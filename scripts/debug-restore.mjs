#!/usr/bin/env node
// 撤去済み観測コードを、撤去コミットの逆 diff から path 限定で戻す。
// 台帳へ SHA を手で持たせず、instrumentation.json の履歴から撤去コミットを探す。

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ledgerPath = join(repoRoot, 'instrumentation.json');
const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const TODO_REASON = 'TODO: なぜ再び必要になったかを書く';

function git(args, options = {}) {
  return execFileSync('git', args, {
    cwd: repoRoot,
    ...(options.encoding === 'buffer' ? {} : { encoding: options.encoding || 'utf8' }),
    input: options.input,
    maxBuffer: 64 * 1024 * 1024,
  });
}

function validDate(value) {
  if (!DATE_RE.test(value)) return false;
  const date = new Date(`${value}T00:00:00Z`);
  return !Number.isNaN(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

function todayUTC() {
  return new Date().toISOString().slice(0, 10);
}

function addDaysUTC(days) {
  const date = new Date(`${todayUTC()}T00:00:00Z`);
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
}

function parseArgs() {
  const args = process.argv.slice(2);
  const idArgs = args.filter(arg => arg.startsWith('--id='));
  const dueArgs = args.filter(arg => arg.startsWith('--due='));
  const dryRun = args.includes('--dry-run');
  const unknown = args.filter(arg => arg !== '--dry-run'
    && !arg.startsWith('--id=') && !arg.startsWith('--due='));
  if (unknown.length > 0 || idArgs.length > 1 || dueArgs.length > 1) {
    console.error('使い方: node scripts/debug-restore.mjs --id=<id> [--dry-run] [--due=YYYY-MM-DD]');
    if (unknown.length > 0) console.error(`BLOCKED: 未知の引数: ${unknown.join(', ')}`);
    return null;
  }
  if (idArgs.length === 0 || idArgs[0] === '--id=') {
    console.error('BLOCKED: --id=<id> が必要です。一括 restore は行いません。');
    return null;
  }
  const id = idArgs[0].slice('--id='.length);
  const due = dueArgs.length > 0 ? dueArgs[0].slice('--due='.length) : addDaysUTC(14);
  if (!validDate(due)) {
    console.error(`BLOCKED: --due=${due} は YYYY-MM-DD の有効な日付ではありません`);
    return null;
  }
  return { id, due, dryRun };
}

function parseLedger(text, source) {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${source} の instrumentation.json を JSON として読めません: ${error.message}`);
  }
}

function showLedger(commitSpec) {
  return parseLedger(git(['show', `${commitSpec}:instrumentation.json`]), commitSpec);
}

function findRemoval(id) {
  const history = git(['log', '--format=%H', '--', 'instrumentation.json'])
    .split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  for (const commit of history) {
    let current;
    let previous;
    try {
      current = showLedger(commit);
      previous = showLedger(`${commit}^`);
    } catch {
      continue;
    }
    const currentEntry = (current.entries || []).find(entry => entry.id === id);
    const previousEntry = (previous.entries || []).find(entry => entry.id === id);
    if (currentEntry?.status === 'removed' && previousEntry?.status !== 'removed') {
      return { commit, previous, previousEntry };
    }
  }
  return null;
}

function repoPath(pathName) {
  if (typeof pathName !== 'string' || pathName.trim() === '') {
    throw new Error('空の path は許可されません');
  }
  const target = resolve(repoRoot, pathName.replaceAll('\\', '/'));
  const rel = relative(repoRoot, target);
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) {
    throw new Error(`リポジトリ外の path は許可されません: ${pathName}`);
  }
  return rel.split(sep).join('/');
}

function diffStat(commit, paths) {
  return git(['diff', '--stat', commit, `${commit}^`, '--', ...paths]);
}

function statusFor(paths) {
  return git(['status', '--porcelain', '--', ...paths, 'instrumentation.json']).trim();
}

function appendTodo(reason) {
  if (typeof reason !== 'string' || reason.trim() === '') return TODO_REASON;
  return reason.includes(TODO_REASON) ? reason : `${reason}\n${TODO_REASON}`;
}

function main() {
  const options = parseArgs();
  if (!options) process.exit(1);
  if (!existsSync(ledgerPath)) {
    console.error('BLOCKED: instrumentation.json が無い。観測コードの台帳は必須。');
    process.exit(1);
  }

  const ledger = parseLedger(readFileSync(ledgerPath, 'utf8'), 'working tree');
  const currentEntry = (ledger.entries || []).find(entry => entry.id === options.id);
  if (!currentEntry) {
    console.error(`BLOCKED: そんな id は無い: ${options.id}`);
    process.exit(1);
  }
  if (currentEntry.status !== 'removed') {
    console.error(`BLOCKED: id=${options.id} は removed ではありません（restore 対象外）`);
    process.exit(1);
  }

  const removal = findRemoval(options.id);
  if (!removal) {
    console.error(`BLOCKED: ${options.id} の撤去コミットを特定できない。台帳を手で編集した場合は restore を諦めて手で書いてください。`);
    process.exit(1);
  }

  const paths = [...new Set([
    ...(removal.previousEntry.files || []),
    ...(removal.previousEntry.sharedFiles || []),
  ].map(repoPath))];
  if (paths.length === 0) {
    console.error(`BLOCKED: ${options.id} の撤去直前台帳に復元対象 path がありません`);
    process.exit(1);
  }

  console.log(`撤去コミット: ${removal.commit}`);
  console.log(`対象 id: ${options.id}`);
  console.log('対象 path:');
  for (const path of paths) console.log(`  ${path}`);
  console.log('逆適用 diffstat:');
  const stat = diffStat(removal.commit, paths).trimEnd();
  console.log(stat || '  (差分なし)');

  if (options.dryRun) return;

  const dirty = statusFor(paths);
  if (dirty !== '') {
    console.error('BLOCKED: 対象 path または instrumentation.json に未コミットの変更があります。何も適用していません。');
    console.error(dirty);
    process.exit(1);
  }

  let patch;
  try {
    patch = git(['diff', '--binary', removal.commit, `${removal.commit}^`, '--', ...paths], { encoding: 'buffer' });
    execFileSync('git', ['apply', '-3', '--'], {
      cwd: repoRoot,
      input: patch,
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch (error) {
    console.error('BLOCKED: 逆 diff の適用に失敗しました。');
    if (error.stderr) console.error(String(error.stderr).trim());
    console.error('コンフリクトは前回の観測点が雛形として残っている状態です。手で合わせてください。');
    process.exit(1);
  }

  currentEntry.status = 'active';
  delete currentEntry.removedIn;
  delete currentEntry.removedOn;
  currentEntry.due = options.due;
  currentEntry.reason = appendTodo(currentEntry.reason);
  writeFileSync(ledgerPath, `${JSON.stringify(ledger, null, 2)}\n`, 'utf8');

  console.log('復元したファイル:');
  for (const path of paths) console.log(`  ${path}`);
  console.log(`台帳: status=active due=${options.due}`);
  console.log('戻した Go ファイルに //go:build maidebug が付いているか確認してください。');
  console.log('web/src/debug/index.ts への import 追加が要る場合があります（check-instrumentation.mjs の第 4 パスが検出します）。');
}

main();

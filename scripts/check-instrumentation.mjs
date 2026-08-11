#!/usr/bin/env node
// 調査用の観測コードがリリースに紛れ込んでいないかを検査する。
//
// 台帳: instrumentation.json（登録の無い観測コードはブロックする）
// 呼び出し元: release の前提チェック / CI
//
// 検査は 3 本立て:
//   1. 台帳に無い観測コードが生えていないか（発見漏れを防ぐ）
//   2. status=active の期限（due）が切れていないか（放置を防ぐ）
//   3. status=removed が本当に消えているか（台帳だけ直して実体が残るのを防ぐ）
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ledgerPath = join(repoRoot, 'instrumentation.json');

// 観測コードの見つけ方。ここに引っかかったファイルは台帳に載っていなければブロック。
const FILE_PATTERNS = [
  /(^|\/)debug_[^/]+\.go$/,
  /(^|\/)[^/]*_trace\.go$/,
  /(^|\/)debug-[^/]+\.ts$/,
];
// 中身に出たら観測コードとみなす印。「消すつもりで書いた」ことを示す語だけを拾う。
const CONTENT_MARKERS = [
  '一時観測',
  '観測専用',
  '観測用エンドポイント',
  '原因が確定したら',
  '原因確定後に撤去',
];
// debug エンドポイントは常に登録必須。
const ENDPOINT_RE = /"\/api\/debug\/[^"]*"/g;

// テストと本台帳・本スクリプト自身は対象外。テストは観測コードの検証にこれらの語を使う。
const EXCLUDE = [
  /_test\.go$/,
  /\.test\.ts$/,
  /^instrumentation\.json$/,
  /^scripts\/check-instrumentation\.mjs$/,
];

function trackedFiles() {
  const out = execFileSync('git', ['ls-files'], { cwd: repoRoot, encoding: 'utf8' });
  return out.split('\n').map(s => s.trim()).filter(Boolean);
}

function cmpVersion(a, b) {
  const pa = String(a).split('.').map(Number);
  const pb = String(b).split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d !== 0) return d;
  }
  return 0;
}

function main() {
  if (!existsSync(ledgerPath)) {
    console.error('BLOCKED: instrumentation.json が無い。観測コードの台帳は必須。');
    process.exit(1);
  }
  const ledger = JSON.parse(readFileSync(ledgerPath, 'utf8'));
  const entries = ledger.entries || [];
  const current = ledger.currentVersion || '0.0.0';

  // 台帳が知っているファイル・エンドポイント（status を問わず）。
  const known = new Set();
  const knownEndpoints = new Set();
  for (const e of entries) {
    for (const f of e.files || []) known.add(f);
    for (const p of e.endpoints || []) knownEndpoints.add(p);
  }

  const problems = [];
  const files = trackedFiles();

  // 1. 台帳に無い観測コード
  for (const f of files) {
    if (EXCLUDE.some(re => re.test(f))) continue;
    let hit = null;
    if (FILE_PATTERNS.some(re => re.test(f))) hit = 'ファイル名が観測コードの命名規則に一致';
    if (!hit && (f.endsWith('.go') || f.endsWith('.ts'))) {
      let body = '';
      try { body = readFileSync(join(repoRoot, f), 'utf8'); } catch { continue; }
      const marker = CONTENT_MARKERS.find(m => body.includes(m));
      if (marker) hit = `本文に「${marker}」がある`;
      for (const m of body.match(ENDPOINT_RE) || []) {
        const ep = m.slice(1, -1);
        if (!knownEndpoints.has(ep)) {
          problems.push(`${f}: debug エンドポイント ${ep} が台帳に無い`);
        }
      }
    }
    if (hit && !known.has(f)) {
      problems.push(`${f}: ${hit} が、instrumentation.json に登録が無い`);
    }
  }

  // 2. 期限切れの active
  for (const e of entries) {
    if (e.status !== 'active') continue;
    if (!e.gate || e.gate === 'always-on') {
      problems.push(`${e.id}: active なのに gate が always-on。opt-in にするか撤去する`);
    }
    if (!e.due) {
      problems.push(`${e.id}: active なのに due が無い。撤去期限を決める`);
      continue;
    }
    if (cmpVersion(current, e.due) >= 0) {
      problems.push(`${e.id}: due=${e.due} に達している（現在 ${current}）。撤去するか due を更新して reason を書き足す`);
    }
  }

  // 3. removed の実体が残っていないか
  const tracked = new Set(files);
  for (const e of entries) {
    if (e.status !== 'removed') continue;
    for (const f of e.files || []) {
      if (tracked.has(f)) problems.push(`${e.id}: status=removed だが ${f} がまだ追跡されている`);
    }
    for (const ep of e.endpoints || []) {
      const still = files.filter(f => (f.endsWith('.go') || f.endsWith('.ts')) && !EXCLUDE.some(re => re.test(f)))
        .filter(f => { try { return readFileSync(join(repoRoot, f), 'utf8').includes(`"${ep}"`); } catch { return false; } });
      if (still.length > 0) problems.push(`${e.id}: status=removed だが ${ep} が ${still.join(', ')} に残っている`);
    }
  }

  if (problems.length > 0) {
    console.error('================================================================');
    console.error(`BLOCKED: 観測コードの検査で ${problems.length} 件の問題を検出`);
    console.error('================================================================');
    for (const p of problems) console.error(`  - ${p}`);
    console.error('');
    console.error('登録する場合は instrumentation.json の entries へ追記する（_comment のルール参照）。');
    process.exit(1);
  }

  const active = entries.filter(e => e.status === 'active');
  console.log(`OK: 観測コードの検査に問題なし（台帳 ${entries.length} 件 / うち active ${active.length} 件）`);
  for (const e of active) console.log(`  active: ${e.id} — gate=${e.gate} due=${e.due}`);
}

main();

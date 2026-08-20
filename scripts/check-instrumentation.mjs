#!/usr/bin/env node
// 調査用の観測コードがリリースに紛れ込んでいないかを検査する。
//
// 台帳: instrumentation.json（登録の無い観測コードはブロックする）
// 呼び出し元: release の前提チェック / CI
//
// 検査は 3 本立て:
//   1. 台帳に無い観測コードが生えていないか（発見漏れを防ぐ）— files と sharedFiles を見る
//   2. status=active の期限（due・YYYY-MM-DD）が切れていないか（放置を防ぐ）
//   3. status=removed が本当に消えているか（台帳だけ直して実体が残るのを防ぐ）— files のみ見る
//   4. web/src/debug/index.ts の登録と台帳の active が一致しているか（両方向）。
//      import があるのに台帳に無い = 未登録の sink が動いている。台帳に active なのに
//      import が無い = sink が登録されず、観測しているつもりで 1 件も記録されない。
//      後者は静かに壊れるので機械で拾う。
//   5. 台帳の files に載る .go が //go:build maidebug を持っているか。
//      これがリリースへ混入しないことの一次担保（二次担保は check-artifact-clean.mjs）。
//      第 1 パスと合わせて「観測 Go ファイル ⊆ maidebug タグ付き」が成り立つ。
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
  // 観測 sink の置き場。probe.ts / index.ts の恒久 2 本だけ EXCLUDE で除く。
  /^web\/src\/debug\//,
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
  // 恒久の足場。観測コードではなく、記録点フックそのもの。
  /^web\/src\/debug\/(?:probe|index)\.ts$/,
  /^internal\/[^/]+\/probe_(?:debug|nodebug)\.go$/,
];

// sink 登録の唯一の入口。第 4 パスがここの import と台帳を突き合わせる。
const DEBUG_INDEX = 'web/src/debug/index.ts';
const DEBUG_INDEX_IMPORT_RE = /^\s*import\s+'\.\/([\w.-]+)\.js';/gm;
const MAIDEBUG_TAG_RE = /^\/\/go:build\s+maidebug\b/m;

function trackedFiles() {
  const out = execFileSync('git', ['ls-files'], { cwd: repoRoot, encoding: 'utf8' });
  return out.split('\n').map(s => s.trim()).filter(Boolean);
}

// due は日付（YYYY-MM-DD）。以前は版数で持ち、台帳の currentVersion と比べていたが、
// その値を進める手順がどこにも無く 2 版ぶん放置された（0.6.0 のまま v0.7.0 を出した）。
// 「手で更新しないと壊れる版数をリポジトリに置かない」という CLAUDE.md の方針にも
// 反していたので、誰も更新しなくても正しい日付比較へ移した。
const DUE_RE = /^\d{4}-\d{2}-\d{2}$/;

// UTC の暦日で比較する。ローカル時刻を使うと作者環境（JST）と CI（UTC）で
// 判定が 1 日ずれるため、どちらでも同じ答えになる側に寄せる。
function todayUTC() {
  return new Date().toISOString().slice(0, 10);
}

// YYYY-MM-DD は辞書順 = 時系列順なので文字列比較でよい。
function daysUntil(due) {
  const ms = Date.parse(`${due}T00:00:00Z`) - Date.parse(`${todayUTC()}T00:00:00Z`);
  return Math.round(ms / 86400000);
}

function main() {
  if (!existsSync(ledgerPath)) {
    console.error('BLOCKED: instrumentation.json が無い。観測コードの台帳は必須。');
    process.exit(1);
  }
  const ledger = JSON.parse(readFileSync(ledgerPath, 'utf8'));
  const entries = ledger.entries || [];

  // 台帳が知っているファイル・エンドポイント（status を問わず）。
  //
  // files と sharedFiles を分けるのは、第 1 パスと第 3 パスで要求が逆になるため。
  //   files       = その観測コード専用のファイル。撤去すればファイルごと消える。
  //                 第 1 パス（登録済みか）と第 3 パス（removed なのに残っていないか）の両方で見る。
  //   sharedFiles = 観測コードが間借りしている共有ファイル（server.go 等）。撤去しても
  //                 ファイル自体は残るので、第 3 パスで見ると removed にした瞬間に必ず落ちる。
  //                 第 1 パスだけで見る。
  //
  // この区別が無かったため「登録すると撤去できない」か「撤去できるが守られない」の
  // 二択になり、ui-input-trace と hub-input-trace の 2 件が files を空にして取り残された。
  const known = new Set();
  const knownEndpoints = new Set();
  for (const e of entries) {
    for (const f of e.files || []) known.add(f);
    for (const f of e.sharedFiles || []) known.add(f);
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
      problems.push(`${e.id}: active なのに due が無い。撤去期限（YYYY-MM-DD）を決める`);
      continue;
    }
    if (!DUE_RE.test(e.due)) {
      problems.push(`${e.id}: due=${e.due} の形式が不正。日付（YYYY-MM-DD）で書く（版数での指定は廃止）`);
      continue;
    }
    const today = todayUTC();
    if (e.due <= today) {
      problems.push(`${e.id}: due=${e.due} を過ぎている（本日 ${today}）。撤去するか due を更新して reason を書き足す`);
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

  // 4. index.ts の sink 登録と台帳の active が一致しているか（両方向）
  const registered = new Set();
  const indexPath = join(repoRoot, DEBUG_INDEX);
  if (existsSync(indexPath)) {
    const body = readFileSync(indexPath, 'utf8');
    DEBUG_INDEX_IMPORT_RE.lastIndex = 0;
    let m;
    while ((m = DEBUG_INDEX_IMPORT_RE.exec(body)) !== null) {
      registered.add(`web/src/debug/${m[1]}.ts`);
    }
  }
  const activeDebugFiles = new Set();
  for (const e of entries) {
    if (e.status !== 'active') continue;
    for (const f of e.files || []) {
      if (f.startsWith('web/src/debug/')) activeDebugFiles.add(f);
    }
  }
  for (const f of registered) {
    if (!activeDebugFiles.has(f)) {
      problems.push(`${DEBUG_INDEX}: ${f} を import しているが、台帳に status=active の登録が無い`);
    }
  }
  for (const f of activeDebugFiles) {
    if (!registered.has(f)) {
      problems.push(`${f}: 台帳では active だが ${DEBUG_INDEX} が import していない（sink が登録されず 1 件も記録されない）`);
    }
  }

  // 5. 台帳の files に載る .go が maidebug タグを持っているか
  //    リリース成果物へ混入しないことの一次担保。
  for (const e of entries) {
    if (e.status !== 'active') continue;
    for (const f of e.files || []) {
      if (!f.endsWith('.go')) continue;
      let body = '';
      try { body = readFileSync(join(repoRoot, f), 'utf8'); } catch { continue; }
      if (!MAIDEBUG_TAG_RE.test(body)) {
        problems.push(`${f}: 観測コードなのに先頭へ //go:build maidebug が無い（既定ビルドへ混入する）`);
      }
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
  for (const e of active) {
    const left = DUE_RE.test(e.due || '') ? `（あと ${daysUntil(e.due)} 日）` : '';
    console.log(`  active: ${e.id} — gate=${e.gate} due=${e.due}${left}`);
  }
}

main();

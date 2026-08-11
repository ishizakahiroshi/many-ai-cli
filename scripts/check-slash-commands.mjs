#!/usr/bin/env node
// resources/slash-commands/*.md がパーサ契約を満たしているかを検査する。
//
// これらの md は main の raw URL から全ユーザーへ実行時配信され、
// internal/hub/slash_cmd_fetch.go の tableRowRe でパースされる。
// 契約を破ると「壊れた」とは分からず、コマンドがピッカーから黙って消える。
//
// 実例（2026-08-11 に発見）: cursor-agent.md に
//   | `/clear` / `/new` / `/new-chat` | 説明 | 説明 |
// のようにエイリアスを 1 行へまとめた行が 6 行あった。tableRowRe はバッククオートを
// 跨げないためこの形は 1 件もマッチせず、/clear /quit /open /run-everything /shell
// /summarize がピッカーに一度も出ていなかった。
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dir = join(repoRoot, 'resources', 'slash-commands');

// internal/hub/slash_cmd_fetch.go の tableRowRe と同じもの。
// ここを変えるときは Go 側と必ず揃える。
const TABLE_ROW = /\|\s*`?(\/[a-z][a-z0-9_-]*)(?:[^`|\n]*)`?\s*\|([^|\n]+)/g;
const ROW_HEAD = /^\| `(\/[a-z][a-z0-9_-]*)` \| ([^|]+) \| ([^|]+) \|$/;

const problems = [];

for (const file of readdirSync(dir).filter(f => f.endsWith('.md')).sort()) {
  const path = join(dir, file);
  const text = readFileSync(path, 'utf8');
  const lines = text.split('\n');
  const rows = lines.filter(l => l.startsWith('| `/'));

  // 1. 行の形（1 行 1 コマンド・3 列）
  const keys = [];
  for (const row of rows) {
    const m = ROW_HEAD.exec(row.trimEnd());
    if (!m) {
      problems.push(`${file}: 行の形が契約に合わない — ${row.trim().slice(0, 70)}`);
      continue;
    }
    keys.push(m[1]);
    for (const col of [m[2], m[3]]) {
      if (/[()[\]（）【】]/.test(col)) {
        problems.push(`${file}: 説明に括弧がある。パーサが括弧内を落とすため情報が消える — ${col.trim().slice(0, 50)}`);
      }
      if (/[`*_[\]]/.test(col.replace(/[[\]]/g, ''))) {
        problems.push(`${file}: 説明に markdown 装飾がある — ${col.trim().slice(0, 50)}`);
      }
    }
  }

  // 2. 実際に Go のパーサが何件拾えるか（拾えない行 = 黙って消えるコマンド）
  TABLE_ROW.lastIndex = 0;
  const parsed = [...text.matchAll(TABLE_ROW)].map(m => m[1]);
  if (parsed.length !== rows.length) {
    problems.push(`${file}: 表は ${rows.length} 行あるがパーサは ${parsed.length} 件しか拾えない。差の ${rows.length - parsed.length} 件はピッカーに出ない`);
  }

  // 3. ABC 順と重複
  const sorted = [...keys].sort();
  for (let i = 0; i < keys.length; i++) {
    if (keys[i] !== sorted[i]) {
      problems.push(`${file}: ABC 順が崩れている — ${keys[i - 1] || '(先頭)'} の次が ${keys[i]}`);
      break;
    }
  }
  const seen = new Set();
  for (const k of keys) {
    if (seen.has(k)) problems.push(`${file}: ${k} が重複している`);
    seen.add(k);
  }
}

if (problems.length > 0) {
  console.error('================================================================');
  console.error(`BLOCKED: スラッシュコマンド定義の検査で ${problems.length} 件の問題を検出`);
  console.error('================================================================');
  for (const p of problems) console.error(`  - ${p}`);
  console.error('');
  console.error('契約: .claude/skills/slash-commands-update/references/provider-sources.md');
  process.exit(1);
}

const counts = readdirSync(dir).filter(f => f.endsWith('.md')).sort().map(f => {
  const n = readFileSync(join(dir, f), 'utf8').split('\n').filter(l => l.startsWith('| `/')).length;
  return `${f.replace('.md', '')}=${n}`;
});
console.log(`OK: スラッシュコマンド定義の検査に問題なし（${counts.join(' ')}）`);

#!/usr/bin/env node
// CLAUDE.md が再び太らないようにするための検査。
//
// なぜ要るか（2026-08-19 制定）:
//
// CLAUDE.md は全 AI セッションで全文がロードされる唯一のファイルである。だから
// ルールが破られるたびに「確実に読まれる場所へ置く」＝ CLAUDE.md へ本文をコピー
// する、が毎回いちばん合理的な対処になってしまう。実際 v0.5.0 の 129 行から
// 2026-08-19 には 269 行へ、5 週間で倍になった。増えた 93 行は日付入りの「制定」
// 節 7 本で、そのすべてに別の場所（設計書 §25・見送り台帳・リリース手順書・
// check-instrumentation.mjs・コード本体）に正本があった。
//
// 昇格（guide から CLAUDE.md へ移す）は制度として存在するのに、降格が存在しない。
// 一方向のラチェットなので放っておくと必ず太る。本スクリプトが降格側の圧力になる。
//
// 検査するのは 3 つ。
//
//   1. 全体の行数が予算内か
//   2. 1 つの節が長すぎないか（長い本文は正本側に置き、ここは索引にする）
//   3. 日付入りの「制定」節が正本の場所を名指ししているか
//
// 予算を超えたときの正しい直し方は「予算を上げる」ではない。次の順に考える。
//
//   a. 機械で強制できるか → 検査スクリプトか Go テストにして、ここは 1 行にする
//   b. 触るファイルが決まっているか → そのファイルの先頭コメントへ移し、ここは 1 行にする
//   c. 提案の可否か → docs/local/reference_declined-directions.md へ行を足し、ここは表の 1 行にする
//
// どれにも当てはまらないものだけが、ここに本文を持ってよい。
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const target = join(repoRoot, 'CLAUDE.md');

// 常時ロード分の行数予算。上げる前に上記 a / b / c を試すこと。
const MAX_LINES = 165;
// 1 節あたりの行数上限。これを超える本文は正本側に置く。
const MAX_SECTION_LINES = 26;
// 「正本を名指ししている」と認める書き方。パス・リンク・スクリプト名・テスト名。
const CANONICAL_HINT =
  /(`[^`]*\.(md|go|ts|mjs|json|yaml|yml)`|\]\([^)]+\)|scripts\/|internal\/|web\/src\/|docs\/|Test[A-Z][A-Za-z0-9_]*)/;

const text = readFileSync(target, 'utf8');
const lines = text.split(/\r?\n/);
const problems = [];

if (lines.length > MAX_LINES) {
  problems.push(
    `CLAUDE.md が ${lines.length} 行で予算 ${MAX_LINES} 行を超えている（超過 ${lines.length - MAX_LINES} 行）。\n` +
      `    予算を上げて通すのは最後の手段。まず本スクリプト冒頭の a / b / c を試すこと。`,
  );
}

/** @type {{title: string, start: number, lines: string[]}[]} */
const sections = [];
let current = null;
lines.forEach((line, i) => {
  if (/^## /.test(line)) {
    if (current) sections.push(current);
    current = { title: line.replace(/^##\s*/, ''), start: i + 1, lines: [] };
    return;
  }
  if (current) current.lines.push(line);
});
if (current) sections.push(current);

for (const section of sections) {
  const body = section.lines.join('\n');
  const count = section.lines.length + 1;
  if (count > MAX_SECTION_LINES) {
    problems.push(
      `CLAUDE.md:${section.start} 「${section.title}」が ${count} 行（上限 ${MAX_SECTION_LINES} 行）。\n` +
        `    本文は正本側へ移し、ここには索引の 1 行だけを残すこと。`,
    );
  }
  // 日付入りの「制定」節は、事故から生まれたルール。必ず正本がどこかにある。
  if (/制定/.test(section.title) && !CANONICAL_HINT.test(body)) {
    problems.push(
      `CLAUDE.md:${section.start} 「${section.title}」が正本の場所を名指ししていない。\n` +
        `    制定ルールは必ずコード・検査スクリプト・台帳のどれかに実体がある。それを書くこと。\n` +
        `    実体が無いなら、それは CLAUDE.md にしか存在しないルールであり、破られても誰も気づけない。`,
    );
  }
}

if (problems.length > 0) {
  console.error('NG: CLAUDE.md の構造検査\n');
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}

console.log(
  `OK: CLAUDE.md は ${lines.length}/${MAX_LINES} 行・節 ${sections.length} 件（最長 ${Math.max(
    ...sections.map((s) => s.lines.length + 1),
  )} 行 / 上限 ${MAX_SECTION_LINES} 行）`,
);

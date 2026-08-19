#!/usr/bin/env node
// 版数の単一ソース（git タグ）が配線されたままかを検査する。
//
// リポジトリ内には版数を持つファイルが 3 系統あるが、どれも CI がタグから
// 上書きするので「古いままなのが正常」である。実際 v0.7.0 の出荷時点で
// npm/many-ai-cli/package.json は 0.3.1、winres/winres.json は 0.3.1.0 の
// ままだったが、配布物はすべて 0.7.0 だった。
//
// この設計は「上書きする側の配線」が消えた瞬間に静かに壊れる。壊れても
// リポジトリ内のファイルは何も変わらないので、次のリリースで 0.3.1 と
// 名乗る成果物が出るまで誰も気づかない。だから配線そのものを検査する。
//
// 逆に「ファイル内の版数がタグと一致するか」は検査しない。一致していない
// ことが正常だからで、そこを揃えにいくと二重管理に戻る。
//
// exit 0 = 配線あり / exit 1 = ブロック。

import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** @type {{file: string, needle: RegExp, what: string}[]} */
const WIRING = [
  {
    file: '.github/workflows/release.yml',
    needle: /scripts\/sync-npm-version\.mjs/,
    what: 'npm/*/package.json の version をタグから焼く配線',
  },
  {
    file: '.goreleaser.yaml',
    needle: /go-winres[\s\S]{0,200}?--file-version=\{\{\s*\.Version\s*\}\}/,
    what: 'winres/*.json の file_version をタグから焼く before hook',
  },
  {
    file: '.goreleaser.yaml',
    needle: /-X\s+main\.version=\{\{\s*\.Version\s*\}\}/,
    what: 'バイナリの version 文字列を焼く ldflags',
  },
];

const problems = [];

for (const { file, needle, what } of WIRING) {
  const path = join(repoRoot, file);
  if (!existsSync(path)) {
    problems.push(`${file} が見つからない（${what}）`);
    continue;
  }
  const text = readFileSync(path, 'utf8');
  if (!needle.test(text)) {
    problems.push(
      `${file} に ${what} が見当たらない。\n` +
        `    版数はタグが単一ソースで、この配線が唯一の反映経路。消すと次のリリースで\n` +
        `    古い版数を名乗る成果物が出るが、リポジトリ内のファイルは何も変わらないので\n` +
        `    出荷するまで気づけない。配線を意図的に変えたなら本スクリプトも同時に直すこと。`,
    );
  }
}

if (problems.length > 0) {
  console.error('NG: 版数の単一ソースの配線が壊れている\n');
  for (const p of problems) console.error(`  - ${p}`);
  console.error('\n正本: CLAUDE.md「設計原則の索引」の版数の行と本スクリプトの冒頭コメント。');
  process.exit(1);
}

console.log(`OK: 版数はタグが単一ソースのまま（配線 ${WIRING.length} 件を確認）`);

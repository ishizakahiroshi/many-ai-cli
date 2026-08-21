#!/usr/bin/env node
// 追跡ファイルにテキストとして壊れたバイトが混入していないかを検査する。
//
// なぜ要るか: AI がファイルを書くとき、Bash の heredoc へ Python 文字列を流す経路で
// エスケープが 1 段落ちることがある。2026-08-21 に 3 回踏んだ。
//   - `\b`（単語境界のつもり）が バックスペース 0x08 として書き込まれ、正規表現が
//     常に false を返した。`.source` を表示しても画面上は `\b` に見えるので、
//     od -c でバイトを見るまで気づけなかった（scripts/check-instrumentation.mjs）
//   - `\s` が `s` になり、`\\n` が実際の改行になった
// 目視では見つからない。バイトで検査するしかない。
//
// 対処の本筋は「正規表現やエスケープを含む編集に heredoc を使わない」ことだが、
// 守られたかどうかを人が確認できないので、混入した結果の方を機械で止める。
//
// 検査は 2 本:
//   1. 制御文字（0x00-0x08 / 0x0B / 0x0C / 0x0E-0x1F）が本文に含まれていないか。
//      タブ(0x09) / LF(0x0A) / CR(0x0D) は正常な文字なので対象外
//   2. .gitattributes が `eol=lf` を明示しているファイルに CRLF が無いか。
//      それ以外のファイルの改行は git の text=auto が正規化するので見ない
//      （working tree は core.autocrlf の設定次第で CRLF になるのが正常）
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

// 本文に出てはいけない制御文字。タブ・LF・CR は除く。
function isForbiddenControlByte(b) {
  if (b === 0x09 || b === 0x0a || b === 0x0d) return false;
  return b <= 0x08 || b === 0x0b || b === 0x0c || (b >= 0x0e && b <= 0x1f);
}

// vendored な第三者コードは対象外。xterm.min.js のように、端末制御シーケンスを
// 本物のバイトで持つのが正しいファイルがある（自分で書いたコードではないので、
// 混入かどうかの判断もできない）。
const EXCLUDE = [
  /^web\/src\/vendor\//,
  /^web\/dist\//,
];

// バイナリは対象外。git がバイナリと判定したものを除く。
function textFiles() {
  const out = execFileSync('git', ['ls-files', '--eol'], { cwd: repoRoot, encoding: 'utf8' });
  const files = [];
  for (const line of out.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    // 例: "i/lf    w/crlf  attr/text=auto  	path/to/file"
    const tab = line.indexOf('\t');
    if (tab < 0) continue;
    const meta = line.slice(0, tab);
    const path = line.slice(tab + 1);
    if (meta.includes('i/-text') || meta.includes('w/-text')) continue; // バイナリ
    if (EXCLUDE.some((re) => re.test(path))) continue;
    files.push({ path, eolLf: /attr\/[^\s]*eol=lf/.test(meta) });
  }
  return files;
}

function main() {
  const problems = [];
  for (const { path, eolLf } of textFiles()) {
    let buf;
    try {
      buf = readFileSync(join(repoRoot, path));
    } catch {
      continue;
    }
    for (let i = 0; i < buf.length; i++) {
      if (!isForbiddenControlByte(buf[i])) continue;
      const line = buf.subarray(0, i).toString('utf8').split('\n').length;
      const hex = buf[i].toString(16).padStart(2, '0');
      const around = JSON.stringify(buf.subarray(Math.max(0, i - 24), i + 8).toString('latin1'));
      problems.push(`${path}:${line}: 制御文字 0x${hex} が本文にある / 前後: ${around}`);
      break; // 1 ファイル 1 件で足りる
    }
    if (eolLf && buf.includes('\r\n')) {
      problems.push(`${path}: .gitattributes が eol=lf を指定しているのに CRLF が含まれる`);
    }
  }

  if (problems.length > 0) {
    console.error('================================================================');
    console.error(`BLOCKED: テキストの健全性検査で ${problems.length} 件の問題を検出`);
    console.error('================================================================');
    for (const p of problems) console.error(`  - ${p}`);
    console.error('');
    console.error('制御文字は「見た目は正しいのに動かない」形で残る。エスケープを含む編集を');
    console.error('heredoc 経由で行っていないか確認し、該当バイトを直接置き換えること。');
    process.exit(1);
  }

  console.log('OK: テキストの健全性検査に問題なし（制御文字の混入なし / eol=lf 指定ファイルの CRLF なし）');
}

main();

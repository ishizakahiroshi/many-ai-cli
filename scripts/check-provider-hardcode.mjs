#!/usr/bin/env node
// provider の id を、コードの中に文字列で直接書き足すことを止める。
//
// 背景:
//   登録した AI CLI（custom provider）が、同梱の 7 本（claude / codex / copilot /
//   cursor-agent / opencode / grok / command-code）と同じ機能で動かなかった原因は、
//   7 本の id を Rust と画面のコードに文字列で書き、provider ごとに処理を分けていたこと
//   だった（docs/local/plan_provider-declarative-capabilities.md）。表（一覧・表示名・
//   URL・権限モード・入力の癖・起動完了の画面文字など）は、定義の「宣言」
//   （rust/assets/provider/declarations.json と、利用者の定義ファイル）から読む形に
//   作り直した。固定が要るときは、コードに書かず定義に宣言する。
//
// 検査内容:
//   rust/src と web/src の .rs / .ts / .html を読み、7 本の id を引用符で囲んだ文字列
//   （"claude" 'codex' `grok` など）の数をファイルごとに数える。次はそもそも数えない。
//     - テスト・fixture・試験データ（tests.rs / *_tests.rs / tests/ / *-fixtures.ts /
//       *.test.ts）、Rust の行頭の #[cfg(test)] が付いた項目（その下の本体は数える）、
//       コメント行
//     - .json（同梱の宣言・試験データ。宣言の置き場そのもの）・.go・.py・.css
//   数えた結果を scripts/provider-hardcode-baseline.json（許可の台帳）と突き合わせる。
//     - 台帳に無いファイルに 1 件でも出た → ブロック
//     - 台帳の件数より増えた → ブロック（行を表示する）
//     - 台帳の件数より減った → 通る。「台帳を下げられる」と表示する（--update で書き直す）
//   台帳の各行には必ず理由を書く（空はエラー）。理由は、plan に残した「意図して各社専用の
//   まま残した場所」の分類に従う（vendor = 各社の画面・API・規則に結びつく処理、
//   identity = 7 本そのものの一覧・予約 id、migration = 旧 Go 版と一致を保つ設定の型、
//   pending = 宣言へ移す途中で残した場所）。
//
// 使い方:
//   node scripts/check-provider-hardcode.mjs            検査（CI）
//   node scripts/check-provider-hardcode.mjs --update   台帳を、いまの件数へ書き直す
//                                                       （件数を下げる時だけ使う。増やす時は理由を足す）
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync, readdirSync, statSync, writeFileSync, existsSync, realpathSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const PROVIDER_IDS = [
  'claude',
  'codex',
  'copilot',
  'cursor-agent',
  'opencode',
  'grok',
  'command-code',
];

const SCAN_ROOTS = ['rust/src', 'web/src'];
const SCAN_EXT = /\.(rs|ts|html)$/;
const BASELINE_PATH = 'scripts/provider-hardcode-baseline.json';

// 数えないファイル（試験・fixture・試験データ・宣言の置き場）。
const SKIP_FILE =
  /(?:^|\/)(?:tests?|tests_[^/]*)\.rs$|_tests?\.rs$|(?:^|\/)tests\/|-fixtures\.ts$|fixtures?\.ts$|\.test\.ts$|\.d\.ts$/;

const quoted = new RegExp('["\'`](' + PROVIDER_IDS.join('|') + ')["\'`]', 'g');

// Rust の文字列・文字リテラル・コメントを 1 つ読み飛ばした先の位置を返す（どれでもなければ i）。
// 括弧の対応を数えるとき、中の括弧を数えないため。
function skipRustToken(s, i) {
  if (s.startsWith('//', i)) {
    const end = s.indexOf('\n', i);
    return end < 0 ? s.length : end;
  }
  if (s.startsWith('/*', i)) {
    let depth = 0;
    for (let j = i; j < s.length; j++) {
      if (s.startsWith('/*', j)) { depth++; j++; }
      else if (s.startsWith('*/', j)) { depth--; j++; if (depth === 0) return j + 1; }
    }
    return s.length;
  }
  const raw = /^b?r(#*)"/.exec(s.slice(i, i + 40));
  if (raw && !/\w/.test(s[i - 1] ?? '')) {
    const end = s.indexOf('"' + raw[1], i + raw[0].length);
    return end < 0 ? s.length : end + 1 + raw[1].length;
  }
  if (s[i] === '"') {
    for (let j = i + 1; j < s.length; j++) {
      if (s[j] === '\\') j++;
      else if (s[j] === '"') return j + 1;
    }
    return s.length;
  }
  // 文字リテラル。ライフタイム（'a）は閉じ引用符が無いので当たらない。
  const ch = /^'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'/u.exec(s.slice(i, i + 16));
  return ch ? i + ch[0].length : i;
}

// 属性の直後の位置 from から始まる 1 項目（mod / fn / use / フィールドなど）の終わりを返す。
function endOfRustItem(s, from) {
  let depth = 0;
  let head = '';
  for (let i = from; i < s.length; i++) {
    const next = skipRustToken(s, i);
    if (next > i) { i = next - 1; continue; }
    const c = s[i];
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') {
      depth--;
      if (depth < 0) return i; // 親の閉じ括弧。この項目はその手前まで
      // 属性（#[...]）の閉じは項目の終わりではない。本体の } で終わる。
      if (depth === 0 && c === '}') return i + 1;
    } else if (depth === 0) {
      if (c === ';') return i + 1;
      // カンマで終わるのはフィールドや腕だけ。宣言の型引数のカンマでは切らない。
      if (c === ',' && !/\b(fn|mod|impl|struct|enum|trait|type|const|static|let|where)\b/.test(head)) {
        return i + 1;
      }
      head += c;
    }
  }
  return s.length;
}

// 行頭の #[cfg(test)] が付いた項目だけを取り除く（行番号は保つ）。以前はそこから下を
// 全部捨てていたので、`#[cfg(test)] mod tests;` を先頭に置いたファイルは本体が検査されなかった。
export function stripRustTestItems(text) {
  const attr = /^#\[cfg\(test\)\]/gm;
  let out = '';
  let kept = 0;
  let m;
  while ((m = attr.exec(text))) {
    const end = endOfRustItem(text, m.index + m[0].length);
    out += text.slice(kept, m.index) + text.slice(m.index, end).replace(/[^\n]+/g, '');
    kept = end;
    attr.lastIndex = end;
  }
  return out + text.slice(kept);
}

export function countHits(relPath, text) {
  const body = relPath.endsWith('.rs') ? stripRustTestItems(text) : text;
  const hits = [];
  const lines = body.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*(\/\/|\*|\/\*)/.test(line)) continue;
    const m = line.match(quoted);
    if (m) for (let k = 0; k < m.length; k++) hits.push({ line: i + 1, text: line.trim() });
  }
  return hits;
}

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
}

export function scan(repoRoot) {
  const result = new Map();
  for (const root of SCAN_ROOTS) {
    const abs = join(repoRoot, root);
    if (!existsSync(abs)) continue;
    const files = [];
    walk(abs, files);
    for (const f of files) {
      const rel = f.slice(repoRoot.length + 1).split(String.fromCharCode(92)).join('/');
      if (!SCAN_EXT.test(rel) || SKIP_FILE.test(rel)) continue;
      const hits = countHits(rel, readFileSync(f, 'utf8'));
      if (hits.length) result.set(rel, hits);
    }
  }
  return result;
}

// 台帳の鍵は、ファイルの相対パス、または「*」（1 階層の中の任意の文字列）を含む型。
// 型に当たったファイルは件数を足して 1 つの枠として扱う（同じ性質のファイルを 1 行にまとめる用）。
function escapeRegExp(s) {
  let out = '';
  for (const ch of s) out += '.+?^${}()|[]'.includes(ch) || ch === String.fromCharCode(92) ? String.fromCharCode(92) + ch : ch;
  return out;
}

function keyMatcher(key) {
  if (!key.includes('*')) return (rel) => rel === key;
  const re = new RegExp(
    '^' + key.split('*').map(escapeRegExp).join('[^/]*') + '$',
  );
  return (rel) => re.test(rel);
}

export function compare(scanned, baselineFiles) {
  const errors = [];
  const shrinkable = [];
  const claimed = new Set();
  for (const [key, allowed] of Object.entries(baselineFiles)) {
    const match = keyMatcher(key);
    let found = 0;
    const hits = [];
    for (const [rel, h] of scanned) {
      if (!match(rel)) continue;
      claimed.add(rel);
      found += h.length;
      for (const x of h) hits.push({ ...x, text: rel + ' ' + x.text });
    }
    if (found > allowed.count) errors.push({ rel: key, allowed: allowed.count, found, hits });
    else if (found < allowed.count) shrinkable.push({ rel: key, allowed: allowed.count, found });
  }
  for (const [rel, hits] of scanned) {
    if (!claimed.has(rel)) errors.push({ rel, allowed: 0, found: hits.length, hits });
  }
  const noReason = Object.entries(baselineFiles)
    .filter(([, v]) => !v.reason || !String(v.reason).trim())
    .map(([rel]) => rel);
  return { errors, shrinkable, noReason };
}

function main() {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const baselineAbs = join(repoRoot, BASELINE_PATH);
  const baseline = JSON.parse(readFileSync(baselineAbs, 'utf8'));
  const scanned = scan(repoRoot);

  if (process.argv.includes('--update')) {
    const next = {};
    const claimed = new Set();
    for (const [key, prev] of Object.entries(baseline.files)) {
      const match = keyMatcher(key);
      let found = 0;
      for (const [rel, hits] of scanned) {
        if (match(rel)) {
          claimed.add(rel);
          found += hits.length;
        }
      }
      if (found > 0) next[key] = { count: Math.min(found, prev.count), reason: prev.reason };
    }
    for (const [rel, hits] of [...scanned].sort((a, b) => a[0].localeCompare(b[0]))) {
      if (!claimed.has(rel)) next[rel] = { count: hits.length, reason: '' };
    }
    baseline.files = next;
    writeFileSync(baselineAbs, JSON.stringify(baseline, null, 2) + '\n');
    console.log('check-provider-hardcode: 台帳を書き直した（件数は増やさない。理由が空の行は埋めること）');
    return;
  }

  const { errors, shrinkable, noReason } = compare(scanned, baseline.files);
  let failed = false;
  if (noReason.length) {
    failed = true;
    console.error('check-provider-hardcode: 台帳に理由が無い行がある:');
    for (const rel of noReason) console.error('  ' + rel);
  }
  if (errors.length) {
    failed = true;
    console.error(
      'check-provider-hardcode: provider の id（' +
        PROVIDER_IDS.join(' / ') +
        '）がコードに直接書き足されている。\n' +
        '  固定が要るときは、コードに書かず定義の宣言（rust/assets/provider/declarations.json）に出す。\n' +
        '  各社専用の処理として本当に残す場合だけ、台帳 ' +
        BASELINE_PATH +
        ' の件数を上げ、理由を書く。',
    );
    for (const e of errors) {
      console.error(`  ${e.rel}: 台帳 ${e.allowed} 件 → いま ${e.found} 件`);
      for (const h of e.hits) console.error(`    L${h.line}: ${h.text}`);
    }
  }
  if (shrinkable.length) {
    console.log('check-provider-hardcode: 台帳より減っている（--update で下げられる）:');
    for (const s of shrinkable) console.log(`  ${s.rel}: ${s.allowed} → ${s.found}`);
  }
  if (failed) process.exit(1);
  let total = 0;
  for (const hits of scanned.values()) total += hits.length;
  console.log(`check-provider-hardcode: OK（${scanned.size} ファイル・${total} 件が台帳の範囲内）`);
}

// 直接実行かどうかは実体パス（realpath）で比べる。Node は実行するファイルの
// symlink・ジャンクションを解いて import.meta.url を作るので、文字列のまま比べると
// ジャンクション・symlink を通したパスで呼んだときに一致せず、
// 何も検査せずに exit 0 で終わってしまう。
function isDirectRun() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isDirectRun()) main();

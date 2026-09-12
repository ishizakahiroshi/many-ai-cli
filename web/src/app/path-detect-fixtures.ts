import assert from 'node:assert/strict';
import test from 'node:test';
import {
  expandLogicalPathLine,
  findPathCandidates,
  isPathContinuationText,
  joinPathWrapRowTexts,
  looksLikePathWrapContinuation,
  type PathWrapRow,
} from './path-detect.js';

function row(text: string, opts: { wrapped?: boolean; width?: number } = {}): PathWrapRow {
  return {
    text,
    isWrapped: !!opts.wrapped,
    contentWidth: opts.width ?? text.length,
  };
}

function getter(rows: PathWrapRow[]) {
  return (i: number) => (i >= 0 && i < rows.length ? rows[i] : null);
}

function combinedPath(rows: PathWrapRow[], index: number, cols?: number): string {
  const { start, end } = expandLogicalPathLine(getter(rows), index, cols);
  return joinPathWrapRowTexts(rows.slice(start, end + 1));
}

const SCREENSHOT_HEAD =
  '実行ファイル: D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\reference_orca-cli-provider-candidates_2026-';
const SCREENSHOT_TAIL = '09-12.md';
const SCREENSHOT_FULL =
  '実行ファイル: D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\reference_orca-cli-provider-candidates_2026-09-12.md';
const SCREENSHOT_PATH =
  'D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\reference_orca-cli-provider-candidates_2026-09-12.md';

test('isPathContinuationText: ファイル名断片は継続、新しい絶対パスは継続ではない', () => {
  assert.equal(isPathContinuationText('09-12.md'), true);
  assert.equal(isPathContinuationText('  09-12.md  '), true);
  assert.equal(isPathContinuationText('.md'), true);
  assert.equal(isPathContinuationText('bar\\baz.md'), true);
  assert.equal(isPathContinuationText('D:\\src\\foo.md'), false);
  assert.equal(isPathContinuationText('/opt/app/foo.md'), false);
  assert.equal(isPathContinuationText('https://example.com/a.md'), false);
  assert.equal(isPathContinuationText('09-12.md is the date'), false);
  assert.equal(isPathContinuationText('変更ファイル:'), false);
});

test('looksLikePathWrapContinuation: スクショのハイフン折り返しを結合する', () => {
  const prev = row(SCREENSHOT_HEAD);
  const next = row(SCREENSHOT_TAIL);
  assert.equal(looksLikePathWrapContinuation(prev, next), true);
});

test('looksLikePathWrapContinuation: 完結した .md の次行は結合しない', () => {
  const prev = row('実行ファイル: D:\\src\\github\\public\\many-ai-cli\\docs\\local\\foo.md');
  const next = row('09-12.md');
  assert.equal(looksLikePathWrapContinuation(prev, next), false);
});

test('looksLikePathWrapContinuation: 拡張子のないディレクトリ行は結合しない', () => {
  const prev = row('cwd: D:\\src\\github\\public\\many-ai-cli');
  const next = row('09-12.md');
  assert.equal(looksLikePathWrapContinuation(prev, next), false);
});

test('expandLogicalPathLine: スクショ経路を 1 本のパスに戻す（先頭行からも継続行からも）', () => {
  const rows = [row(SCREENSHOT_HEAD), row(SCREENSHOT_TAIL)];
  assert.equal(combinedPath(rows, 0), SCREENSHOT_FULL);
  assert.equal(combinedPath(rows, 1), SCREENSHOT_FULL);
  const found = findPathCandidates(combinedPath(rows, 0));
  assert.equal(found.length, 1);
  assert.equal(found[0].text, SCREENSHOT_PATH);
});

test('expandLogicalPathLine: 3 行に割れたハイフン折り返しも結合する', () => {
  const rows = [
    row('実行ファイル: D:\\src\\foo\\reference_orca-cli-provider-candidates_2026-'),
    row('09-'),
    row('12.md'),
  ];
  assert.equal(combinedPath(rows, 0), '実行ファイル: D:\\src\\foo\\reference_orca-cli-provider-candidates_2026-09-12.md');
  assert.equal(combinedPath(rows, 2), '実行ファイル: D:\\src\\foo\\reference_orca-cli-provider-candidates_2026-09-12.md');
});

test('expandLogicalPathLine: Unix 絶対パスのハイフン折り返し', () => {
  const rows = [
    row('open /opt/app/docs/reference_orca-cli-provider-candidates_2026-'),
    row('09-12.md'),
  ];
  const combined = combinedPath(rows, 0);
  const found = findPathCandidates(combined);
  assert.equal(found.length, 1);
  assert.equal(found[0].text, '/opt/app/docs/reference_orca-cli-provider-candidates_2026-09-12.md');
});

test('expandLogicalPathLine: 相対パスのハイフン折り返し', () => {
  const rows = [
    row('変更ファイル: docs/local/reference/reference_orca-cli-provider-candidates_2026-'),
    row('09-12.md'),
  ];
  const combined = combinedPath(rows, 0);
  const found = findPathCandidates(combined);
  assert.equal(found.some((c) => c.text.endsWith('reference_orca-cli-provider-candidates_2026-09-12.md')), true);
});

test('expandLogicalPathLine: xterm isWrapped はハイフン無しでも結合する', () => {
  const rows = [
    row('D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\file', { wrapped: false, width: 80 }),
    row('name.md', { wrapped: true, width: 7 }),
  ];
  const combined = combinedPath(rows, 1, 80);
  assert.equal(combined, 'D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\filename.md');
});

test('expandLogicalPathLine: 行幅いっぱいの途中折れはハイフン無しでも結合する', () => {
  const head = 'D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\fi';
  const rows = [
    row(head, { width: 80 }),
    row('lename.md', { width: 9 }),
  ];
  const combined = combinedPath(rows, 0, 80);
  assert.equal(combined, 'D:\\src\\github\\public\\many-ai-cli\\docs\\local\\reference\\filename.md');
});

test('expandLogicalPathLine: 継続行の行頭インデントはパスに入れない', () => {
  const rows = [
    row('実行ファイル: D:\\src\\foo\\bar_2026-'),
    row('    09-12.md'),
  ];
  const combined = combinedPath(rows, 0);
  assert.equal(combined, '実行ファイル: D:\\src\\foo\\bar_2026-09-12.md');
});

test('expandLogicalPathLine: 次行が新しい Windows パスなら結合しない', () => {
  const rows = [
    row('実行ファイル: D:\\src\\foo.md'),
    row('D:\\src\\bar.md'),
  ];
  assert.equal(combinedPath(rows, 0), '実行ファイル: D:\\src\\foo.md');
  assert.equal(expandLogicalPathLine(getter(rows), 0).end, 0);
});

test('expandLogicalPathLine: ラベル行とは結合しない', () => {
  const rows = [
    row('実行ファイル: D:\\src\\foo.md'),
    row('変更ファイル: D:\\src\\bar.md'),
  ];
  assert.equal(expandLogicalPathLine(getter(rows), 0).end, 0);
});

const PREVIEWABLE_EXT_RE = /\.(md|markdown|ts|tsx|json|jsonl)$/i;

test('findPathCandidates: バッククォート囲みの Windows パスから閉じ記号を残さない', () => {
  const wrapped = '`D:\\src\\github\\public\\many-ai-cli\\web\\src\\app\\path-detect.ts`';
  const found = findPathCandidates(wrapped);
  assert.equal(found.length, 1);
  assert.equal(found[0].text, 'D:\\src\\github\\public\\many-ai-cli\\web\\src\\app\\path-detect.ts');
  assert.equal(PREVIEWABLE_EXT_RE.test(found[0].text), true);
});

test('findPathCandidates: 単引用符と二重引用符の囲みも拡張子で終わる', () => {
  const single = "'D:\\src\\foo\\bar.ts'";
  const double = '"D:\\src\\foo\\bar.md"';
  const s = findPathCandidates(single);
  const d = findPathCandidates(double);
  assert.equal(s.length, 1);
  assert.equal(s[0].text, 'D:\\src\\foo\\bar.ts');
  assert.equal(PREVIEWABLE_EXT_RE.test(s[0].text), true);
  assert.equal(d.length, 1);
  assert.equal(d[0].text, 'D:\\src\\foo\\bar.md');
  assert.equal(PREVIEWABLE_EXT_RE.test(d[0].text), true);
});

test('findPathCandidates: フッター複数行のバッククォート囲みパスを全部拾う', () => {
  const text = [
    '変更ファイル:',
    '`D:\\src\\github\\public\\many-ai-cli\\web\\src\\app\\path-detect.ts`',
    '`D:\\src\\github\\public\\many-ai-cli\\web\\src\\app\\path-links.ts`',
    '`D:\\src\\github\\public\\many-ai-cli\\CHANGELOG.md`',
  ].join('\n');
  const found = findPathCandidates(text);
  assert.equal(found.length, 3);
  assert.ok(found.every((c) => PREVIEWABLE_EXT_RE.test(c.text)));
  assert.ok(found.every((c) => !c.text.includes('`')));
});

test('findPathCandidates: Unix 絶対パスは従来どおりバッククォートで切る', () => {
  const found = findPathCandidates('`/opt/app/docs/foo.md`');
  assert.equal(found.length, 1);
  assert.equal(found[0].text, '/opt/app/docs/foo.md');
  assert.equal(PREVIEWABLE_EXT_RE.test(found[0].text), true);
});

test('isPathContinuationText: 閉じバッククォート付きの断片も継続と見る', () => {
  assert.equal(isPathContinuationText('09-13.md`'), true);
  assert.equal(isPathContinuationText('  09-13.md`  '), true);
});

test('expandLogicalPathLine: バッククォート囲みのハイフン折り返しも 1 本に戻す', () => {
  const rows = [
    row('実行ファイル: `D:\\src\\foo\\bugfix_example_2026-'),
    row('09-13.md`'),
  ];
  const combined = combinedPath(rows, 0);
  assert.equal(combined, '実行ファイル: `D:\\src\\foo\\bugfix_example_2026-09-13.md`');
  const found = findPathCandidates(combined);
  assert.equal(found.length, 1);
  assert.equal(found[0].text, 'D:\\src\\foo\\bugfix_example_2026-09-13.md');
  assert.equal(PREVIEWABLE_EXT_RE.test(found[0].text), true);
});

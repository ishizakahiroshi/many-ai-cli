import assert from 'node:assert/strict';
import test from 'node:test';
import {
  expandLogicalPathLine,
  findPathCandidates,
  joinPathWrapRowTexts,
  type PathWrapRow,
} from './path-detect.js';
import {
  findUrlCandidates,
  isHttpUrl,
  looksLikeLinkWrapContinuation,
  looksLikeUrlWrapContinuation,
  splitUrlPieces,
  trimUrlCandidate,
} from './url-detect.js';

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

function combinedLink(rows: PathWrapRow[], index: number, cols?: number): string {
  const { start, end } = expandLogicalPathLine(getter(rows), index, cols, looksLikeLinkWrapContinuation);
  return joinPathWrapRowTexts(rows.slice(start, end + 1));
}

function urls(text: string): string[] {
  return findUrlCandidates(text).map((c) => c.url);
}

test('findUrlCandidates: 前後に空白がある URL をそのまま拾う', () => {
  assert.deepEqual(urls('see https://example.com/a/b for details'), ['https://example.com/a/b']);
});

test('findUrlCandidates: 直後に日本語や全角記号が続いても URL に含めない', () => {
  assert.deepEqual(urls('詳細はhttps://example.com/docs。次の文'), ['https://example.com/docs']);
  assert.deepEqual(urls('（https://example.com/x）を開く'), ['https://example.com/x']);
  assert.deepEqual(urls('https://example.com/raw/123 です'), ['https://example.com/raw/123']);
  assert.deepEqual(urls('https://example.com/pathを参照'), ['https://example.com/path']);
});

test('findUrlCandidates: 対応の取れない閉じ括弧は落とし、URL の中で閉じた括弧は残す', () => {
  assert.deepEqual(urls('(https://example.com/x)'), ['https://example.com/x']);
  assert.deepEqual(urls('[link](https://example.com/y)'), ['https://example.com/y']);
  assert.deepEqual(urls('https://en.wikipedia.org/wiki/Foo_(bar)'), ['https://en.wikipedia.org/wiki/Foo_(bar)']);
  assert.deepEqual(urls('(see https://en.wikipedia.org/wiki/Foo_(bar))'), ['https://en.wikipedia.org/wiki/Foo_(bar)']);
});

test('findUrlCandidates: 文末の句読点・強調記号・引用符を URL に含めない', () => {
  assert.deepEqual(urls('Visit https://example.com/path.'), ['https://example.com/path']);
  assert.deepEqual(urls('https://example.com/?q=1&r=2, next'), ['https://example.com/?q=1&r=2']);
  assert.deepEqual(urls('**https://example.com**'), ['https://example.com']);
  assert.deepEqual(urls("'https://example.com/a'"), ['https://example.com/a']);
  assert.deepEqual(urls('"https://example.com/b"'), ['https://example.com/b']);
  assert.deepEqual(urls('`https://example.com/c`'), ['https://example.com/c']);
  assert.deepEqual(urls('<https://example.com/d>'), ['https://example.com/d']);
});

test('findUrlCandidates: 位置は元の文字列上の範囲を指す', () => {
  const text = 'A https://example.com/one と https://example.org/two。';
  const found = findUrlCandidates(text);
  assert.equal(found.length, 2);
  for (const c of found) assert.equal(text.slice(c.start, c.end), c.url);
  assert.deepEqual(found.map((c) => c.url), ['https://example.com/one', 'https://example.org/two']);
});

test('findUrlCandidates: URL でないものは拾わない', () => {
  assert.deepEqual(urls('https:// だけ'), []);
  assert.deepEqual(urls('xhttps://example.com'), []);
  assert.deepEqual(urls('ftp://example.com/file'), []);
  assert.deepEqual(urls('example.com/path'), []);
});

test('findUrlCandidates: 大文字の scheme と http も拾う', () => {
  assert.deepEqual(urls('HTTPS://EXAMPLE.COM/A'), ['HTTPS://EXAMPLE.COM/A']);
  assert.deepEqual(urls('http://127.0.0.1:47777/?token=x'), ['http://127.0.0.1:47777/?token=x']);
});

test('trimUrlCandidate / isHttpUrl', () => {
  assert.equal(trimUrlCandidate('https://example.com/a).'), 'https://example.com/a');
  assert.equal(isHttpUrl('https://example.com'), true);
  assert.equal(isHttpUrl('javascript:alert(1)'), false);
  assert.equal(isHttpUrl('https://'), false);
});

test('splitUrlPieces: ハイライトで断片に割れた URL は、またいだ断片ごとに同じ行き先で返す', () => {
  const values = ['url = "', 'https:', '//example.com/a', '"; // done'];
  const pieces = splitUrlPieces(values);
  assert.deepEqual(pieces[0], []);
  assert.deepEqual(pieces[1], [{ from: 0, to: 6, url: 'https://example.com/a' }]);
  assert.deepEqual(pieces[2], [{ from: 0, to: 15, url: 'https://example.com/a' }]);
  assert.deepEqual(pieces[3], []);
});

test('splitUrlPieces: 1 つの断片に URL が 2 本あっても両方返す', () => {
  const values = ['see https://example.com/a and https://example.org/b.', ' tail'];
  const pieces = splitUrlPieces(values);
  assert.deepEqual(pieces[0].map((p) => values[0].slice(p.from, p.to)), ['https://example.com/a', 'https://example.org/b']);
  assert.deepEqual(pieces[1], []);
});

test('splitUrlPieces: 断片の境目で URL が終わっても次の断片へ持ち越さない', () => {
  const values = ['https://example.com/a', ' next https://example.org/'];
  const pieces = splitUrlPieces(values);
  assert.deepEqual(pieces[0], [{ from: 0, to: 21, url: 'https://example.com/a' }]);
  assert.deepEqual(pieces[1], [{ from: 6, to: 26, url: 'https://example.org/' }]);
});

test('splitUrlPieces: URL が無ければすべて空', () => {
  assert.deepEqual(splitUrlPieces(['plain', ' text']), [[], []]);
  assert.deepEqual(splitUrlPieces([]), []);
});

test('looksLikeUrlWrapContinuation: ハイフンで折り返した URL を結合する', () => {
  const prev = row('  詳細: https://example.com/files/review_no12-q3-');
  const next = row('  screen-change_2026-09-25.html');
  assert.equal(looksLikeUrlWrapContinuation(prev, next), true);
});

test('looksLikeUrlWrapContinuation: 行幅いっぱいで切れた URL を結合する', () => {
  const prev = row('https://example.com/abcdef', { width: 80 });
  const next = row('ghi/jkl');
  assert.equal(looksLikeUrlWrapContinuation(prev, next, 80), true);
});

test('looksLikeUrlWrapContinuation: 途中で切れた形でない URL の次の行は結合しない', () => {
  assert.equal(looksLikeUrlWrapContinuation(row('https://example.com/a'), row('next'), 80), false);
});

test('looksLikeUrlWrapContinuation: 次の行が文章・新しい URL なら結合しない', () => {
  const prev = row('https://example.com/abcdef', { width: 80 });
  assert.equal(looksLikeUrlWrapContinuation(prev, row('次の文です'), 80), false);
  assert.equal(looksLikeUrlWrapContinuation(prev, row('and more text'), 80), false);
  assert.equal(looksLikeUrlWrapContinuation(prev, row('https://example.org/'), 80), false);
  assert.equal(looksLikeUrlWrapContinuation(prev, row(''), 80), false);
});

test('expandLogicalPathLine + looksLikeLinkWrapContinuation: 割れた URL を 1 本に戻す（どの行からでも）', () => {
  const rows = [
    row('  詳細: https://example.com/files/review_no12-q3-'),
    row('  screen-change_2026-09-25.html'),
  ];
  for (const index of [0, 1]) {
    assert.deepEqual(urls(combinedLink(rows, index)), ['https://example.com/files/review_no12-q3-screen-change_2026-09-25.html']);
  }
});

test('expandLogicalPathLine + looksLikeLinkWrapContinuation: 3 行に割れた URL も結合する', () => {
  const rows = [
    row('  https://example.com/aaaa-'),
    row('  bbbb-'),
    row('  cccc.html'),
  ];
  for (const index of [0, 1, 2]) {
    assert.deepEqual(urls(combinedLink(rows, index)), ['https://example.com/aaaa-bbbb-cccc.html']);
  }
});

test('looksLikeLinkWrapContinuation: パスの折り返しは従来どおり結合する', () => {
  const rows = [
    row('実行ファイル: D:\\work\\foo\\reference_example_2026-'),
    row('09-12.md'),
  ];
  const found = findPathCandidates(combinedLink(rows, 0));
  assert.equal(found.length, 1);
  assert.equal(found[0].text, 'D:\\work\\foo\\reference_example_2026-09-12.md');
});

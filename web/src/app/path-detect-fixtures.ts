import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {
  ABS_WIN_PATH_RE,
  boundedRowGetter,
  expandLogicalPathLine,
  findPathCandidates,
  isPathContinuationText,
  joinPathWrapRowTexts,
  looksLikePathWrapContinuation,
  trimTerminalPathCandidate,
  trimWindowsPathCandidate,
  type PathWrapRow,
} from './path-detect.js';

async function checkPathCandidateInChild(check: string): Promise<void> {
  const moduleUrl = new URL(import.meta.url.endsWith('.ts') ? './path-detect.ts' : './path-detect.js', import.meta.url).href;
  // A native regexp can delay Worker.terminate() in Bun. A separate process
  // remains killable even while the regexp engine is still evaluating a match.
  const { stdout } = await promisify(execFile)(process.execPath, ['--eval', `
    (async () => {
      const m = await import(${JSON.stringify(moduleUrl)});
      ${check}
      console.log('ok');
    })().catch(error => { console.error(error); process.exitCode = 1; });
  `], { timeout: 5000, killSignal: 'SIGKILL', windowsHide: true });
  assert.equal(stdout.trim(), 'ok');
}

test('Windows paths: whitespace runs preserve raw captures and the next drive boundary', () => {
  const previous = /([A-Za-z]:[\\/](?:(?!\s+[A-Za-z]:[\\/])[^\x00-\x1f<>:"|?*(`])+)/g;
  const parts = [' ', '\t', '\n', '\r', '\u00a0', '\u2028', 'あ', 'a', ')', ' C:/next', '\tD:\\next', 'x:/next'];
  const capture = (re: RegExp, text: string) => {
    re.lastIndex = 0;
    const found: Array<[number, string, string, number]> = [];
    let match: RegExpExecArray | null;
    while ((match = re.exec(text)) !== null) found.push([match.index, match[0], match[1], re.lastIndex]);
    return found;
  };
  for (const first of parts) for (const second of parts) {
    const text = 'C:/file' + first + second + ' D:/last';
    assert.deepEqual(capture(ABS_WIN_PATH_RE, text), capture(previous, text));
  }
});

test('Windows path descriptions: whitespace and line breaks retain their suffix meaning', () => {
  for (const [input, expected] of [
    ['C:/file   説明', 'C:/file'],
    ['C:/file\n  説明', 'C:/file'],
    ['C:/file あ\nend', 'C:/file あ\nend'],
    ['C:/file あ\n tail 説明', 'C:/file あ\n tail'],
    ['C:/file\u00a0Ａ', 'C:/file'],
    ['C:/file   end', 'C:/file   end'],
    ['C:/file   A', 'C:/file'],
    ['C:/file\nA', 'C:/file'],
    ['C:/file A\n', 'C:/file A\n'],
  ]) assert.equal(trimWindowsPathCandidate(input), expected);
});

test('path detection: long whitespace and wrap punctuation complete without blocking', { timeout: 10000 }, async () => {
  await checkPathCandidateInChild(`
      const text = 'C:/file' + ' '.repeat(100000) + 'end';
      m.ABS_WIN_PATH_RE.lastIndex = 0;
      const match = m.ABS_WIN_PATH_RE.exec(text);
      if (match?.[1] !== text || m.trimWindowsPathCandidate(text) !== text
        || m.isPathContinuationText(')'.repeat(100000) + 'x') !== false) {
        throw new Error('path detection changed the candidate');
      }
  `);
});

test('path suffix: trailing punctuation is removed and internal punctuation is preserved', () => {
  assert.equal(trimTerminalPathCandidate('  /work/report.md ) ] ;  '), '/work/report.md');
  assert.equal(trimTerminalPathCandidate('D:/work/(draft)))x.txt'), 'D:/work/(draft)))x.txt');
  assert.equal(trimWindowsPathCandidate('D:/work/report.md )\n'), 'D:/work/report.md )\n');
  assert.equal(trimWindowsPathCandidate('D:/work/report.md )'), 'D:/work/report.md');
});

test('path suffix: long internal punctuation completes without blocking the caller', { timeout: 10000 }, async () => {
  await checkPathCandidateInChild(`
      const text = 'D:/work/' + ')'.repeat(10000) + 'x';
      if (m.trimTerminalPathCandidate(text) !== text || m.trimWindowsPathCandidate(text) !== text) {
        throw new Error('internal punctuation was modified');
      }
  `);
});

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

test('boundedRowGetter: 範囲外で先頭へ巻き戻るバッファでも全行折り返しで止まる', () => {
  // xterm の環状バッファと同じく、範囲外の index が先頭側の行を返す読み出し
  const length = 29;
  let reads = 0;
  const cyclicRead = (_index: number) => {
    reads++;
    if (reads > 10_000) throw new Error('row reader did not terminate');
    return row('x'.repeat(10), { wrapped: true, width: 10 });
  };
  assert.equal(cyclicRead(length + 3).isWrapped, true);
  const getRow = boundedRowGetter(length, (i) => cyclicRead(i % length));
  assert.equal(getRow(length), null);
  assert.equal(getRow(-1), null);
  assert.deepEqual(expandLogicalPathLine(getRow, 5, 80), { start: 0, end: length - 1 });
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

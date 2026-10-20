import { test, expect } from 'bun:test';
import {
  MEMO_EXPORT_FORMATS,
  buildMemoExport,
  memoExportFileName,
  parseMemoImport,
  planMemoImport,
  selectMemosForExport,
} from '../src/app/memo-export';
import type { ImportItem, MemoExportLabels } from '../src/app/memo-export';
import type { Memo } from '../src/app/memo-model';

function memo(over: Partial<Memo>): Memo {
  return {
    id: 'x', text: 't', project: '', done: false,
    created_at: '2026-09-28T00:00:00Z', updated_at: '2026-09-28T00:00:00Z',
    ...over,
  };
}

const labels: MemoExportLabels = {
  title: (count, date) => `作業メモ（${date} 出力・${count} 件）`,
  unclassified: '未分類',
  images: (count) => `（画像 ${count} 枚）`,
  doneMark: '完了',
};

// ローカル時刻で組み立てる（日付・時刻の整形がタイムゾーンに左右されないことも兼ねて確かめる）。
const NOW = new Date(2026, 9, 20, 12, 15, 30);

const APP = 'D:\\work\\sample-app';
const DOCS = 'D:\\work\\sample-docs';

// 見本の 3 件（未完了）。/resume のほうが新しい。
const resume = memo({
  id: 'a', text: '/resume', project: APP,
  created_at: '2026-10-19T10:00:00Z', updated_at: '2026-10-19T10:00:00Z',
});
const review = memo({
  id: 'b', text: 'レビュー待ちの PR を確認する\nログは docs/notes.md を見る', project: APP,
  created_at: '2026-10-18T10:00:00Z', updated_at: '2026-10-18T10:00:00Z',
  images: ['1.png', '2.png'],
});
const terms = memo({
  id: 'c', text: '手順書の用語をそろえる', project: DOCS,
  created_at: '2026-10-17T10:00:00Z', updated_at: '2026-10-17T10:00:00Z',
});
const finished = memo({
  id: 'd', text: '古い対応', project: APP, done: true,
  created_at: '2026-10-10T10:00:00Z', updated_at: '2026-10-11T10:00:00Z', done_at: '2026-10-11T10:00:00Z',
});
const all = [terms, finished, review, resume];

test('selectMemosForExport: 選択があればそれだけ（完了済みでも）', () => {
  expect(selectMemosForExport(all, new Set(['d', 'c']), false).map((m) => m.id)).toEqual(['c', 'd']);
  expect(selectMemosForExport(all, new Set(['d']), true).map((m) => m.id)).toEqual(['d']);
});

test('selectMemosForExport: 選択が無ければ includeDone で未完了のみ／全件、元の順を保つ', () => {
  expect(selectMemosForExport(all, new Set(), false).map((m) => m.id)).toEqual(['c', 'b', 'a']);
  expect(selectMemosForExport(all, new Set(), true).map((m) => m.id)).toEqual(['c', 'd', 'b', 'a']);
});

test('text 形式は見本と一致する', () => {
  const r = buildMemoExport(selectMemosForExport(all, new Set(), false), 'text', labels, { now: NOW });
  expect(r.content).toBe(
    [
      '作業メモ（2026-10-20 出力・3 件）',
      '',
      '■ sample-app',
      '・/resume',
      '・レビュー待ちの PR を確認する',
      '  ログは docs/notes.md を見る',
      '  （画像 2 枚）',
      '',
      '■ sample-docs',
      '・手順書の用語をそろえる',
      '',
    ].join('\n'),
  );
  expect(r.count).toBe(3);
  expect(r.extension).toBe('txt');
  expect(r.mime).toContain('text/plain');
});

test('markdown 形式は見本と一致する', () => {
  const r = buildMemoExport(selectMemosForExport(all, new Set(), false), 'markdown', labels, { now: NOW });
  expect(r.content).toBe(
    [
      '# 作業メモ（2026-10-20 出力・3 件）',
      '',
      '## sample-app',
      '- [ ] /resume',
      '- [ ] レビュー待ちの PR を確認する',
      '  ログは docs/notes.md を見る',
      '  （画像 2 枚）',
      '',
      '## sample-docs',
      '- [ ] 手順書の用語をそろえる',
      '',
    ].join('\n'),
  );
  expect(r.extension).toBe('md');
  expect(r.mime).toContain('text/markdown');
});

test('完了メモは markdown が [x]、text が [完了] で、未完了の後ろに並ぶ', () => {
  const md = buildMemoExport(all, 'markdown', labels, { now: NOW }).content;
  expect(md).toContain('- [ ] レビュー待ちの PR を確認する\n  ログは docs/notes.md を見る\n  （画像 2 枚）\n- [x] 古い対応\n');
  const tx = buildMemoExport(all, 'text', labels, { now: NOW }).content;
  expect(tx).toContain('（画像 2 枚）\n・[完了] 古い対応\n');
});

test('activeProject のグループが先頭になる', () => {
  const r = buildMemoExport([resume, terms], 'text', labels, { now: NOW, activeProject: DOCS });
  expect(r.content.indexOf('■ sample-docs')).toBeLessThan(r.content.indexOf('■ sample-app'));
});

test('グループ見出しは末尾のフォルダ名（\\ / の両方・末尾の区切りは無視）、空 project は未分類', () => {
  const memos = [
    memo({ id: '1', text: 'a', project: 'D:\\work\\win-dir\\', updated_at: '2026-10-03T00:00:00Z' }),
    memo({ id: '2', text: 'b', project: '/work/sample/posix-dir', updated_at: '2026-10-02T00:00:00Z' }),
    memo({ id: '3', text: 'c', project: '', updated_at: '2026-10-04T00:00:00Z' }),
  ];
  const content = buildMemoExport(memos, 'text', labels, { now: NOW }).content;
  expect(content).toContain('■ win-dir\n');
  expect(content).toContain('■ posix-dir\n');
  expect(content.indexOf('■ 未分類')).toBeGreaterThan(content.indexOf('■ posix-dir'));
});

test('複数行本文の空行は字下げなしの空行、画像のみのメモは 1 行目が画像表記', () => {
  const memos = [
    memo({ id: '1', text: '一行目\n\n三行目\n', project: APP, created_at: '2026-10-02T00:00:00Z' }),
    memo({ id: '2', text: '', project: APP, images: ['x.png'], created_at: '2026-10-01T00:00:00Z' }),
  ];
  const content = buildMemoExport(memos, 'text', labels, { now: NOW }).content;
  expect(content).toBe(
    ['作業メモ（2026-10-20 出力・2 件）', '', '■ sample-app', '・一行目', '', '  三行目', '・（画像 1 枚）', ''].join('\n'),
  );
});

test('0 件でも見出しだけを返す（3 形式とも例外にしない）', () => {
  expect(buildMemoExport([], 'text', labels, { now: NOW }).content).toBe('作業メモ（2026-10-20 出力・0 件）\n');
  expect(buildMemoExport([], 'markdown', labels, { now: NOW }).content).toBe('# 作業メモ（2026-10-20 出力・0 件）\n');
  const json = buildMemoExport([], 'json', labels, { now: NOW });
  expect(JSON.parse(json.content).memos).toEqual([]);
  expect(json.count).toBe(0);
});

test('json 形式: version・exported_at・作成日時の昇順・フラット・id なし', () => {
  const r = buildMemoExport(all, 'json', labels, { now: NOW });
  expect(r.content.endsWith('}\n')).toBe(true);
  expect(r.content).toContain('\n  "version": 1,\n');
  expect(r.extension).toBe('json');
  expect(r.count).toBe(4);
  const data = JSON.parse(r.content);
  expect(data.version).toBe(1);
  expect(data.exported_at).toMatch(/^2026-10-20T12:15:30[+-]\d{2}:\d{2}$/);
  expect(data.memos.map((m: { text: string }) => m.text)).toEqual([
    '古い対応',
    '手順書の用語をそろえる',
    'レビュー待ちの PR を確認する\nログは docs/notes.md を見る',
    '/resume',
  ]);
  expect(data.memos[0].done_at).toBe('2026-10-11T10:00:00Z');
  expect('done_at' in data.memos[1]).toBe(false);
  expect('id' in data.memos[0]).toBe(false);
  expect(data.memos[2].images).toEqual(['1.png', '2.png']);
  expect(data.memos[1].images).toEqual([]);
});

test('json を取り込み直すと text / project / done が戻り、作成順が保たれる', () => {
  const r = buildMemoExport(all, 'json', labels, { now: NOW });
  const parsed = parseMemoImport(r.content);
  expect(parsed.ok).toBe(true);
  if (!parsed.ok) return;
  expect(parsed.skippedEmpty).toBe(0);
  expect(parsed.items).toEqual([
    { text: '古い対応', project: APP, done: true },
    { text: '手順書の用語をそろえる', project: DOCS, done: false },
    { text: 'レビュー待ちの PR を確認する\nログは docs/notes.md を見る', project: APP, done: false },
    { text: '/resume', project: APP, done: false },
  ]);
});

test('memoExportFileName は日時を 2 桁詰めで入れ、拡張子は形式で決まる', () => {
  expect(memoExportFileName('markdown', new Date(2026, 9, 20, 12, 15, 30))).toBe('memos_20261020_121530.md');
  expect(memoExportFileName('text', new Date(2026, 0, 2, 3, 4, 5))).toBe('memos_20260102_030405.txt');
  expect(memoExportFileName('json', new Date(2026, 0, 2, 3, 4, 5))).toBe('memos_20260102_030405.json');
  expect(MEMO_EXPORT_FORMATS).toEqual(['markdown', 'text', 'json']);
});

test('parseMemoImport: 空・壊れた JSON・形が違うものは理由付きで失敗する', () => {
  expect(parseMemoImport('  \n ')).toEqual({ ok: false, reason: 'empty' });
  expect(parseMemoImport('{oops')).toEqual({ ok: false, reason: 'invalid_json' });
  expect(parseMemoImport('"text"')).toEqual({ ok: false, reason: 'wrong_shape' });
  expect(parseMemoImport('42')).toEqual({ ok: false, reason: 'wrong_shape' });
  expect(parseMemoImport('null')).toEqual({ ok: false, reason: 'wrong_shape' });
  expect(parseMemoImport('{"version":1}')).toEqual({ ok: false, reason: 'wrong_shape' });
  expect(parseMemoImport('{"memos":"x"}')).toEqual({ ok: false, reason: 'wrong_shape' });
});

test('parseMemoImport: 配列直置きを受け、型の違う project / done は既定値にする', () => {
  const raw = JSON.stringify([
    { text: ' a ', project: 5, done: 'yes' },
    { text: 'b', project: APP, done: true },
  ]);
  const parsed = parseMemoImport(raw);
  expect(parsed).toEqual({
    ok: true,
    skippedEmpty: 0,
    items: [
      { text: 'a', project: '', done: false },
      { text: 'b', project: APP, done: true },
    ],
  });
});

test('parseMemoImport: 本文が空・文字列でない要素は捨てて数え、1 件も無くても ok', () => {
  const parsed = parseMemoImport(JSON.stringify({ memos: [{ text: '  ' }, { text: 3 }, null, 'x', { text: 'ok' }] }));
  expect(parsed).toEqual({ ok: true, skippedEmpty: 4, items: [{ text: 'ok', project: '', done: false }] });
  expect(parseMemoImport('[]')).toEqual({ ok: true, skippedEmpty: 0, items: [] });
  expect(parseMemoImport('{"memos":[{"text":""}]}')).toEqual({ ok: true, skippedEmpty: 1, items: [] });
});

// 取り込み先: project が APP のものは session 1、DOCS のものは session 2、それ以外は未分類（null）。
function resolver(item: ImportItem) {
  if (item.project === APP) return { sessionId: 1, belongs: (p: string) => p === APP };
  if (item.project === DOCS) return { sessionId: 2, belongs: (p: string) => p === DOCS };
  return { sessionId: null, belongs: (p: string) => p === '' };
}

test('planMemoImport: 同じ本文で同じ取り込み先の既存メモは重複、別プロジェクトなら重複にしない', () => {
  const existing = [memo({ id: 'e1', text: '  /resume ', project: APP })];
  const items: ImportItem[] = [
    { text: '/resume', project: APP, done: false },
    { text: '/resume', project: DOCS, done: false },
  ];
  const plan = planMemoImport(items, existing, resolver);
  expect(plan.duplicates).toBe(1);
  expect(plan.create).toEqual([{ item: items[1], sessionId: 2 }]);
});

test('planMemoImport: 完了済みの既存メモとも重複扱いにする', () => {
  const existing = [memo({ id: 'e1', text: '古い対応', project: APP, done: true })];
  const plan = planMemoImport([{ text: '古い対応', project: APP, done: false }], existing, resolver);
  expect(plan).toEqual({ create: [], duplicates: 1 });
});

test('planMemoImport: 今回の候補同士の重複は最初の 1 件だけ作る（sessionId が同じとき）', () => {
  const items: ImportItem[] = [
    { text: 'same', project: APP, done: false },
    { text: ' same ', project: APP, done: true },
    { text: 'same', project: DOCS, done: false },
    { text: 'same', project: '', done: false },
    { text: 'same', project: '/other', done: false },
  ];
  const plan = planMemoImport(items, [], resolver);
  expect(plan.create.map((c) => c.sessionId)).toEqual([1, 2, null]);
  expect(plan.create[0].item).toBe(items[0]);
  expect(plan.duplicates).toBe(2);
});

test('planMemoImport: 取り込み先が未分類（sessionId null）同士も重複判定する', () => {
  const existing = [memo({ id: 'e1', text: 'free', project: '' })];
  const plan = planMemoImport(
    [{ text: 'free', project: '', done: false }, { text: 'new', project: '', done: false }],
    existing,
    resolver,
  );
  expect(plan.duplicates).toBe(1);
  expect(plan.create.map((c) => c.item.text)).toEqual(['new']);
});

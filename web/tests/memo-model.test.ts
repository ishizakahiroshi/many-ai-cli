import { test, expect } from 'bun:test';
import {
  countOpenMemosForCwd,
  countOpenMemosForProject,
  cwdMatchesMemoProject,
  findOpenMemoByText,
  formatMemoTimestamp,
  memoWasEdited,
  pruneMemoSelection,
  groupMemosByProject,
  memoPathType,
  openMemoCountForCwd,
  pickMemoImageFiles,
  resolveMemoPathTarget,
  setSharedMemoCache,
  sortDoneMemos,
  sortOpenMemos,
  totalOpenMemoCount,
} from '../src/app/memo-model';
import type { Memo } from '../src/app/memo-model';

function memo(over: Partial<Memo>): Memo {
  return {
    id: 'x', text: 't', project: '', done: false,
    created_at: '2026-09-28T00:00:00Z', updated_at: '2026-09-28T00:00:00Z',
    ...over,
  };
}

test('active project group comes first regardless of update time', () => {
  const memos = [
    memo({ id: '1', project: '/repo/a', updated_at: '2026-09-01T00:00:00Z' }),
    memo({ id: '2', project: '/repo/b', updated_at: '2026-09-27T00:00:00Z' }),
  ];
  const groups = groupMemosByProject(memos, '/repo/a');
  expect(groups.map((g) => g.project)).toEqual(['/repo/a', '/repo/b']);
});

test('non-active groups are ordered by latest update, newest first', () => {
  const memos = [
    memo({ id: '1', project: '/repo/old', updated_at: '2026-09-01T00:00:00Z' }),
    memo({ id: '2', project: '/repo/new', updated_at: '2026-09-27T00:00:00Z' }),
  ];
  const groups = groupMemosByProject(memos, '');
  expect(groups.map((g) => g.project)).toEqual(['/repo/new', '/repo/old']);
});

test('unclassified (empty project) is always last, even if it is the active project', () => {
  const memos = [
    memo({ id: '1', project: '', updated_at: '2026-09-27T00:00:00Z' }),
    memo({ id: '2', project: '/repo/a', updated_at: '2026-09-01T00:00:00Z' }),
  ];
  const groups = groupMemosByProject(memos, '');
  expect(groups.map((g) => g.project)).toEqual(['/repo/a', '']);
});

test('each group splits into open and done', () => {
  const memos = [
    memo({ id: '1', project: '/repo/a', done: false }),
    memo({ id: '2', project: '/repo/a', done: true }),
  ];
  const groups = groupMemosByProject(memos, '');
  expect(groups[0].open.map((m) => m.id)).toEqual(['1']);
  expect(groups[0].done.map((m) => m.id)).toEqual(['2']);
});

test('sortOpenMemos shows the most recently created memo first', () => {
  const memos = [
    memo({ id: 'old', created_at: '2026-09-01T00:00:00Z' }),
    memo({ id: 'new', created_at: '2026-09-27T00:00:00Z' }),
  ];
  expect(sortOpenMemos(memos).map((m) => m.id)).toEqual(['new', 'old']);
});

test('sortDoneMemos shows the most recently finished memo first', () => {
  const memos = [
    memo({ id: 'a', done: true, done_at: '2026-09-01T00:00:00Z' }),
    memo({ id: 'b', done: true, done_at: '2026-09-27T00:00:00Z' }),
  ];
  expect(sortDoneMemos(memos).map((m) => m.id)).toEqual(['b', 'a']);
});

test('countOpenMemosForProject counts only undone memos in that project (unclassified via empty string)', () => {
  const memos = [
    memo({ id: '1', project: '/repo/a', done: false }),
    memo({ id: '2', project: '/repo/a', done: true }),
    memo({ id: '3', project: '', done: false }),
  ];
  expect(countOpenMemosForProject(memos, '/repo/a')).toBe(1);
  expect(countOpenMemosForProject(memos, '')).toBe(1);
  expect(countOpenMemosForProject(memos, '/repo/missing')).toBe(0);
});

test('resolveMemoPathTarget leaves absolute paths untouched', () => {
  expect(resolveMemoPathTarget('C:\\repo\\plan.md', '/anything')).toBe('C:\\repo\\plan.md');
  expect(resolveMemoPathTarget('/repo/plan.md', 'D:\\other')).toBe('/repo/plan.md');
});

test('resolveMemoPathTarget resolves a relative path against the memo project', () => {
  expect(resolveMemoPathTarget('docs\\local\\plan_x.md', 'D:\\dev\\repo')).toBe('D:\\dev\\repo\\docs\\local\\plan_x.md'); // doxguard: allow D:\\dev\\
});

test('resolveMemoPathTarget returns the candidate as-is when the memo has no project (unclassified)', () => {
  expect(resolveMemoPathTarget('docs/local/plan_x.md', '')).toBe('docs/local/plan_x.md');
});

// C4: セッションの cwd と memo.project の突き合わせ。project_id（Hub が非同期に解決する
// git root）ではなく cwd で判定する理由は memo-model.ts のコメント参照。
test('cwdMatchesMemoProject matches when cwd equals the project (case/backslash insensitive)', () => {
  expect(cwdMatchesMemoProject('D:\\dev\\repo', 'D:\\dev\\repo')).toBe(true); // doxguard: allow D:\\dev\\
  expect(cwdMatchesMemoProject('D:\\Dev\\Repo', 'd:\\dev\\repo')).toBe(true); // doxguard: allow D:\\Dev\\ doxguard: allow d:\\dev\\
  expect(cwdMatchesMemoProject('/repo/a', '/repo/a')).toBe(true);
});

test('cwdMatchesMemoProject matches a cwd nested under the project (mixed separators)', () => {
  expect(cwdMatchesMemoProject('D:\\dev\\repo\\sub\\dir', 'D:\\dev\\repo')).toBe(true); // doxguard: allow D:\\dev\\
  expect(cwdMatchesMemoProject('/repo/a/sub', '/repo/a')).toBe(true);
});

test('cwdMatchesMemoProject does not match a sibling directory with a shared prefix', () => {
  expect(cwdMatchesMemoProject('/repo/a-other', '/repo/a')).toBe(false);
});

test('cwdMatchesMemoProject is false for empty cwd or empty (unclassified) project', () => {
  expect(cwdMatchesMemoProject('', '/repo/a')).toBe(false);
  expect(cwdMatchesMemoProject('/repo/a', '')).toBe(false);
});

test('countOpenMemosForCwd counts only undone memos whose project matches the cwd', () => {
  const memos = [
    memo({ id: '1', project: '/repo/a', done: false }),
    memo({ id: '2', project: '/repo/a', done: true }),
    memo({ id: '3', project: '/repo/b', done: false }),
  ];
  expect(countOpenMemosForCwd(memos, '/repo/a/sub')).toBe(1);
  expect(countOpenMemosForCwd(memos, '/repo/b')).toBe(1);
  expect(countOpenMemosForCwd(memos, '/repo/missing')).toBe(0);
});

// C4: session-list.ts はこの共有キャッシュ経由で件数を読む（memo-panel.ts を import
// できないため。memo-model.ts のコメント参照）。
test('setSharedMemoCache feeds openMemoCountForCwd and totalOpenMemoCount', () => {
  setSharedMemoCache([
    memo({ id: '1', project: '/repo/a', done: false }),
    memo({ id: '2', project: '/repo/a', done: true }),
    memo({ id: '3', project: '/repo/b', done: false }),
  ]);
  expect(openMemoCountForCwd('/repo/a')).toBe(1);
  expect(openMemoCountForCwd('/repo/b')).toBe(1);
  expect(openMemoCountForCwd('/repo/missing')).toBe(0);
  expect(totalOpenMemoCount()).toBe(2);
  setSharedMemoCache([]);
  expect(totalOpenMemoCount()).toBe(0);
});

test('pickMemoImageFiles keeps only the four image types the Hub accepts', () => {
  const files = [
    { type: 'image/png' }, { type: 'image/jpeg' }, { type: 'image/gif' }, { type: 'image/webp' },
    { type: 'image/svg+xml' }, { type: 'text/plain' }, { type: '' },
  ];
  expect(pickMemoImageFiles(files).map((f) => f.type)).toEqual(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
});

test('findOpenMemoByText matches an open memo with the same text, ignoring surrounding spaces', () => {
  const path = 'D:\\repo\\docs\\local\\plan_x.md';
  const memos = [
    memo({ id: 'done', text: path, done: true }),
    memo({ id: 'open', text: `  ${path}\n` }),
  ];
  expect(findOpenMemoByText(memos, path)?.id).toBe('open');
  expect(findOpenMemoByText(memos, ` ${path} `)?.id).toBe('open');
});

test('findOpenMemoByText ignores done memos, other text and empty input', () => {
  const path = '/repo/docs/plan_x.md';
  const memos = [memo({ id: 'done', text: path, done: true }), memo({ id: 'other', text: `${path} C3 から` })];
  expect(findOpenMemoByText(memos, path)).toBeUndefined();
  expect(findOpenMemoByText(memos, '   ')).toBeUndefined();
});

// C2: 作業メモ内のパスをファイルかフォルダか推測する（showPathPopup の pathType に渡す）。
test('memoPathType treats a path with an extension as a file', () => {
  expect(memoPathType('D:\\x\\plan_a.md')).toBe('file');
  expect(memoPathType('docs\\local\\plan_x.md')).toBe('file');
  expect(memoPathType('/repo/docs/plan_x.md')).toBe('file');
});

test('memoPathType treats a path without an extension as a directory, except known file names', () => {
  expect(memoPathType('F:\\nursery\\writing\\card_strategy_note_bundle')).toBe('dir');
  expect(memoPathType('D:\\x\\Makefile')).toBe('file');
  expect(memoPathType('/repo/Dockerfile')).toBe('file');
});

test('memoPathType treats a trailing separator as a directory', () => {
  expect(memoPathType('D:\\x\\sub\\')).toBe('dir');
  expect(memoPathType('/repo/sub/')).toBe('dir');
});

test('formatMemoTimestamp omits the year only for the current year', () => {
  const now = new Date(2026, 9, 20, 12, 0);
  expect(formatMemoTimestamp(new Date(2026, 9, 19, 22, 5).toISOString(), now)).toBe('10/19 22:05');
  expect(formatMemoTimestamp(new Date(2025, 0, 3, 9, 7).toISOString(), now)).toBe('2025/1/3 09:07');
  expect(formatMemoTimestamp('', now)).toBe('');
  expect(formatMemoTimestamp('garbage', now)).toBe('');
});

test('memoWasEdited ignores updates within a minute of creation', () => {
  expect(memoWasEdited({ created_at: '2026-10-19T10:00:00Z', updated_at: '2026-10-19T10:00:30Z' })).toBe(false);
  expect(memoWasEdited({ created_at: '2026-10-19T10:00:00Z', updated_at: '2026-10-19T10:05:00Z' })).toBe(true);
});

test('pruneMemoSelection drops ids that no longer exist', () => {
  const memos = [{ id: 'a' }, { id: 'b' }] as never[];
  expect(Array.from(pruneMemoSelection(new Set(['a', 'x']), memos))).toEqual(['a']);
});

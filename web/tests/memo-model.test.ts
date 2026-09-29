import { test, expect } from 'bun:test';
import {
  countOpenMemosForCwd,
  countOpenMemosForProject,
  cwdMatchesMemoProject,
  groupMemosByProject,
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
  expect(resolveMemoPathTarget('docs\\local\\plan_x.md', 'D:\\dev\\repo')).toBe('D:\\dev\\repo\\docs\\local\\plan_x.md');
});

test('resolveMemoPathTarget returns the candidate as-is when the memo has no project (unclassified)', () => {
  expect(resolveMemoPathTarget('docs/local/plan_x.md', '')).toBe('docs/local/plan_x.md');
});

// C4: セッションの cwd と memo.project の突き合わせ。project_id（Hub が非同期に解決する
// git root）ではなく cwd で判定する理由は memo-model.ts のコメント参照。
test('cwdMatchesMemoProject matches when cwd equals the project (case/backslash insensitive)', () => {
  expect(cwdMatchesMemoProject('D:\\dev\\repo', 'D:\\dev\\repo')).toBe(true);
  expect(cwdMatchesMemoProject('D:\\Dev\\Repo', 'd:\\dev\\repo')).toBe(true);
  expect(cwdMatchesMemoProject('/repo/a', '/repo/a')).toBe(true);
});

test('cwdMatchesMemoProject matches a cwd nested under the project (mixed separators)', () => {
  expect(cwdMatchesMemoProject('D:\\dev\\repo\\sub\\dir', 'D:\\dev\\repo')).toBe(true);
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

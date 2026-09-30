// detached-view-mode-fixtures.ts — 別窓の URL の組み立てと読み取りの往復を確かめる。
// plan: docs/local/plan_file-preview-popout-window.md C1
//
// ファイルの別窓は、パスをそのまま URL に載せて新しい窓へ渡す。Windows のパスは空白・#・&・%・
// 日本語を含みうるので、組み立てた URL を読み戻して同じパスになることを固定する。
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildDetachedFileUrl,
  buildDetachedTabUrl,
  detachedPopupFeatures,
  parseDetachedTabSearch,
} from './detached-view-mode.js';

function roundTrip(path: string, sessionId?: number | null, cwd?: string) {
  const url = buildDetachedFileUrl(path, sessionId, cwd);
  assert.ok(url.startsWith('/?'), url);
  return parseDetachedTabSearch(url.slice(1));
}

test('file window URL: Windows paths with spaces, #, &, %, + and Japanese survive the round trip', () => {
  const paths = [
    'D:\\dev\\my project\\docs\\local\\plan_a b.md',
    'C:\\work\\#1 & 2\\100% done+more.md',
    'D:\\資料\\議事録 2026-09-30（水）.md',
    '\\\\fileserver\\share\\team docs\\readme.md',
    '/srv/project/notes/a?b=c.md',
  ];
  for (const path of paths) {
    const got = roundTrip(path, 12, 'D:\\dev\\my project');
    assert.deepEqual(got, { tab: 'file', sessionId: 12, path, cwd: 'D:\\dev\\my project' }, path);
  }
});

test('file window URL: session and cwd are optional', () => {
  assert.deepEqual(roundTrip('D:\\a.md'), { tab: 'file', sessionId: 0, path: 'D:\\a.md', cwd: '' });
  assert.deepEqual(roundTrip('D:\\a.md', 0, ''), { tab: 'file', sessionId: 0, path: 'D:\\a.md', cwd: '' });
});

test('file window URL: tab=file without a path is not a detached window', () => {
  assert.equal(parseDetachedTabSearch('?view=detached-tab&tab=file&session=3'), null);
  assert.equal(parseDetachedTabSearch('?view=detached-tab&tab=file&path='), null);
});

test('other detached tabs keep working and ignore path / cwd', () => {
  assert.deepEqual(
    parseDetachedTabSearch(buildDetachedTabUrl('git', 3).slice(1)),
    { tab: 'git', sessionId: 3, path: '', cwd: '' },
  );
  assert.deepEqual(
    parseDetachedTabSearch('?view=detached-tab&tab=memo&path=D%3A%5Ca.md&cwd=D%3A%5C'),
    { tab: 'memo', sessionId: 0, path: '', cwd: '' },
  );
  assert.equal(parseDetachedTabSearch('?view=detached-tab&tab=unknown'), null);
  assert.equal(parseDetachedTabSearch('?view=detached-grid&tab=file&path=D%3A%5Ca.md'), null);
});

test('popup features always carry a size so the browser opens a window, not a tab', () => {
  // node には screen が無いので、画面の大きさでの切り詰めは起きない
  assert.equal(detachedPopupFeatures(), 'popup,width=960,height=1000');
  assert.equal(detachedPopupFeatures({ width: 700.4, height: 820.6 }), 'popup,width=700,height=821');
  assert.equal(detachedPopupFeatures({ width: 0, height: 500 }), 'popup,width=960,height=1000');
  assert.equal(detachedPopupFeatures(null), 'popup,width=960,height=1000');
});

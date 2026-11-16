import assert from 'node:assert/strict';
import test from 'node:test';
import {
  OWN_CREATION_WINDOW_MS,
  consumeOwnCreation,
  idsToAnnounce,
  launchCommandLine,
  undoFailureMessageKey,
} from './provider-added-notice-model.js';

test('undoFailureMessageKey: 版の食い違い（409）は「足したあとに変わった」、それ以外は取り消せなかった', () => {
  assert.equal(undoFailureMessageKey({ kind: 'http', status: 409 }), 'provider_added_undo_changed');
  assert.equal(undoFailureMessageKey({ kind: 'http', status: 404 }), 'provider_added_undo_failed');
  assert.equal(undoFailureMessageKey({ kind: 'http', status: 500 }), 'provider_added_undo_failed');
  assert.equal(undoFailureMessageKey({ kind: 'network' }), 'provider_added_undo_failed');
});

const created = (...ids: string[]) => ({ ids, action: 'created' });

test('idsToAnnounce: created の id をそのまま知らせる', () => {
  assert.deepEqual(idsToAnnounce(created('fixture-cli'), 1000, new Map()), ['fixture-cli']);
  assert.deepEqual(idsToAnnounce(created('a', 'b'), 1000, new Map()), ['a', 'b']);
});

test('idsToAnnounce: created 以外（更新・削除・無効化・復元）は知らせない', () => {
  for (const action of ['updated', 'removed', 'disabled', 'restored', 'reset', 'recovered', '']) {
    assert.deepEqual(idsToAnnounce({ ids: ['fixture-cli'], action }, 1000, new Map()), [], action);
  }
});

test('idsToAnnounce: この画面が足した id は知らせず、印は 1 回で外れる', () => {
  const marks = new Map([['mine', 1000]]);
  assert.deepEqual(idsToAnnounce(created('mine', 'theirs'), 2000, marks), ['theirs']);
  assert.equal(marks.has('mine'), false);
  // 同じ id が別の経路で足されたら、2 回目は知らせる。
  assert.deepEqual(idsToAnnounce(created('mine'), 3000, marks), ['mine']);
});

test('consumeOwnCreation: 期限を過ぎた印は外すだけで、足した扱いにしない', () => {
  const marks = new Map([['stale', 1000]]);
  assert.equal(consumeOwnCreation('stale', 1000 + OWN_CREATION_WINDOW_MS + 1, marks), false);
  assert.equal(marks.size, 0);
  const fresh = new Map([['fresh', 1000]]);
  assert.equal(consumeOwnCreation('fresh', 1000 + OWN_CREATION_WINDOW_MS, fresh), true);
  assert.equal(consumeOwnCreation('unknown', 2000, new Map()), false);
});

test('launchCommandLine: 実行ファイルと引数を 1 行にし、空白を含む語は引用する', () => {
  assert.equal(
    launchCommandLine({ executable: 'fixture-agent', args: ['--interactive', 'two words'] }),
    'fixture-agent --interactive "two words"',
  );
  assert.equal(launchCommandLine({ executable: 'fixture-agent' }), 'fixture-agent');
});

test('launchCommandLine: 崩れた中身は空にし、長すぎる行は切る', () => {
  assert.equal(launchCommandLine(undefined), '');
  assert.equal(launchCommandLine('x'), '');
  assert.equal(launchCommandLine({ executable: 3, args: 'nope' }), '');
  assert.equal(launchCommandLine({ executable: 'a', args: ['b', 7, null] }), 'a b');
  const long = launchCommandLine({ executable: 'x'.repeat(400) });
  assert.equal(long.length, 161);
  assert.ok(long.endsWith('…'));
});

// project-view-memory-fixtures.ts — 箱ごとの記憶が「保存値をそのまま信じない」ことを固定する。
//
// ここが赤くなったら、直すのはテストではなく実装。とくに次の 2 つは、画面が黙って
// おかしくなる側の失敗なので、テストを緩めて通してはいけない。
//
//  1. 未知のタブ名が既定（terminal）へ落ちる。落ちないと setActiveTab が何もせず、
//     「復元したのに前のタブのまま」という読み方のできない状態になる。
//  2. 記憶していたセッションが消えていたら先頭へ落ち、そのときだけ fallback が立つ。
//     fallback をいつも立てると、復元のたびに保存が走って記憶が潰れる。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c4_state-memory.md

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_TAB_NAME,
  PROJECT_KEY_MAX_LEN,
  PROJECT_VIEWS_MAX,
  VALID_TAB_NAME_LIST,
  isValidTabName,
  normalizeTabName,
  pickRestoreSession,
  sanitizeProjectViews,
  withProjectView,
} from './project-view-memory.js';

// 合成データ。実在のパス・リポジトリ名は書かない（公開ファイルの層 1 防御）。
const BOX_A = '/src/box-alpha';
const BOX_B = '/src/box-bravo';

test('タブ名は既知の 9 種だけを通す', () => {
  assert.equal(VALID_TAB_NAME_LIST.length, 9);
  for (const name of VALID_TAB_NAME_LIST) assert.ok(isValidTabName(name));
  for (const bogus of ['', 'Terminal', 'workbench', 'chat ', null, undefined, 3, {}]) {
    assert.equal(isValidTabName(bogus), false);
  }
});

test('未知のタブ名は捨てずに既定へ落とす', () => {
  assert.equal(normalizeTabName('git'), 'git');
  assert.equal(normalizeTabName('workbench'), DEFAULT_TAB_NAME);
  assert.equal(normalizeTabName(undefined), DEFAULT_TAB_NAME);
  assert.equal(normalizeTabName({ tab: 'git' }), DEFAULT_TAB_NAME);
});

test('オブジェクトでない保存値はまるごと捨てる', () => {
  for (const bogus of [null, undefined, 'x', 3, [], [{ session_id: 1 }]]) {
    assert.equal(sanitizeProjectViews(bogus), null);
  }
  assert.deepEqual(sanitizeProjectViews({}), {});
});

test('壊れたエントリだけを落として残りは使う', () => {
  const got = sanitizeProjectViews({
    [BOX_A]: { session_id: 7, tab: 'git' },
    [BOX_B]: { session_id: 0, tab: 'git' },        // ID が不正
    '/src/box-charlie': { session_id: 2 },          // タブ無し → 既定へ
    '/src/box-delta': { session_id: '5', tab: 'x' }, // 文字列 ID は拾い、未知のタブは落とす
    '/src/box-echo': 'not-an-object',
    '': { session_id: 1, tab: 'git' },              // 空キー
    ['k'.repeat(PROJECT_KEY_MAX_LEN + 1)]: { session_id: 1, tab: 'git' },
  });
  assert.deepEqual(got, {
    [BOX_A]: { session_id: 7, tab: 'git' },
    '/src/box-charlie': { session_id: 2, tab: DEFAULT_TAB_NAME },
    '/src/box-delta': { session_id: 5, tab: DEFAULT_TAB_NAME },
  });
});

test('件数の上限を超えたぶんは捨てる', () => {
  const raw: Record<string, unknown> = {};
  for (let i = 0; i < PROJECT_VIEWS_MAX + 25; i++) {
    raw[`/src/box-${i}`] = { session_id: i + 1, tab: 'terminal' };
  }
  const got = sanitizeProjectViews(raw);
  assert.equal(Object.keys(got as object).length, PROJECT_VIEWS_MAX);
});

test('書き足しは元の map を書き換えない', () => {
  const before = { [BOX_A]: { session_id: 7, tab: 'git' as const } };
  const after = withProjectView(before, BOX_B, { session_id: 9, tab: 'chat' });
  assert.deepEqual(before, { [BOX_A]: { session_id: 7, tab: 'git' } });
  assert.deepEqual(after[BOX_B], { session_id: 9, tab: 'chat' });
  assert.deepEqual(after[BOX_A], { session_id: 7, tab: 'git' });
});

test('書き足しでも壊れた値は入らない', () => {
  const before = { [BOX_A]: { session_id: 7, tab: 'git' as const } };
  assert.deepEqual(withProjectView(before, BOX_B, { session_id: 'x', tab: 'chat' }), before);
  assert.deepEqual(withProjectView(before, '', { session_id: 9, tab: 'chat' }), before);
  // 未知のタブ名は既定へ落ちる（エントリごと捨てない）。
  assert.deepEqual(
    withProjectView(before, BOX_B, { session_id: 9, tab: 'workbench' })[BOX_B],
    { session_id: 9, tab: DEFAULT_TAB_NAME },
  );
});

test('上限に達していても更新対象の箱は必ず残る', () => {
  let views = {};
  for (let i = 0; i < PROJECT_VIEWS_MAX; i++) {
    views = withProjectView(views, `/src/box-${i}`, { session_id: i + 1, tab: 'terminal' });
  }
  assert.equal(Object.keys(views).length, PROJECT_VIEWS_MAX);
  const next = withProjectView(views, BOX_A, { session_id: 42, tab: 'git' });
  assert.equal(Object.keys(next).length, PROJECT_VIEWS_MAX);
  assert.deepEqual(next[BOX_A], { session_id: 42, tab: 'git' });
  // 追い出されるのはいちばん古いもの。
  assert.equal('/src/box-0' in next, false);
});

test('記憶しているセッションが生きていれば、そのまま開く（保存は上書きしない）', () => {
  const pick = pickRestoreSession({ session_id: 3, tab: 'git' }, [1, 3, 5]);
  assert.deepEqual(pick, { sessionId: 3, fallback: false, tab: 'git' });
});

test('記憶しているセッションが消えていたら先頭へ落ち、そのときだけ上書きする', () => {
  const pick = pickRestoreSession({ session_id: 99, tab: 'git' }, [5, 1, 3]);
  // 並べ替えない＝渡された順の先頭。
  assert.deepEqual(pick, { sessionId: 5, fallback: true, tab: 'git' });
});

test('記憶が無い箱は先頭を開き、タブは変えない（tab=null）', () => {
  // ここを DEFAULT_TAB_NAME にすると、multi タブを開いたまま箱を渡り歩く使い方が
  // 「まだ記憶の無い箱」を開いた瞬間に毎回途切れる。
  assert.deepEqual(pickRestoreSession(undefined, [4, 2]), {
    sessionId: 4, fallback: true, tab: null,
  });
  assert.equal(pickRestoreSession(null, [4]).tab, null);
  assert.equal(pickRestoreSession('bogus', [4]).tab, null);
});

test('箱が空なら何もしない', () => {
  assert.deepEqual(pickRestoreSession({ session_id: 3, tab: 'git' }, []), {
    sessionId: null, fallback: false, tab: 'git',
  });
});

test('復元側でも未知のタブ名を弾く（保存経路が 1 本だけとは限らない）', () => {
  const pick = pickRestoreSession({ session_id: 3, tab: 'workbench' }, [3]);
  assert.deepEqual(pick, { sessionId: 3, fallback: false, tab: DEFAULT_TAB_NAME });
});

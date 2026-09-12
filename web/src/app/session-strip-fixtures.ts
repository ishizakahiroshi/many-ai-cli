// session-strip-fixtures.ts — セッション帯のブランチ名省略を固定する。
//
// ここが赤くなったら、直すのはテストではなく実装。帯は「頭が同じで尻だけ違う」
// ブランチ（relay の子が 2 本以上動いているとき）を区別できることが役目なので、
// 尻を落とす省略（CSS の text-overflow）へ戻してはいけない。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c2_session-strip.md

import assert from 'node:assert/strict';
import test from 'node:test';
import { abbreviateBranchName } from './session-strip-branch.js';

// 合成データ。実在のブランチ名は書かない（公開ファイルの層 1 防御）。
const SHORT = 'develop';
const LONG_A = 'feature/session-strip-alpha';
const LONG_B = 'feature/session-strip-bravo';

test('上限以下のブランチ名はそのまま返る', () => {
  assert.equal(abbreviateBranchName(SHORT, 18), SHORT);
  assert.equal(abbreviateBranchName('123456789012345678', 18), '123456789012345678');
});

test('上限を超えたら真ん中を省き、頭と尻の両方を残す', () => {
  const out = abbreviateBranchName(LONG_A, 18);
  assert.ok(out.includes('…'), '省略記号が入る');
  assert.ok(LONG_A.startsWith(out.split('…')[0]), '頭が元と一致する');
  assert.ok(LONG_A.endsWith(out.split('…')[1]), '尻が元と一致する');
});

test('頭が同じで尻だけ違うブランチは、省略後も区別できる', () => {
  // 尻を落とす省略（text-overflow）だとここが同じ文字列になる。
  assert.notEqual(abbreviateBranchName(LONG_A, 18), abbreviateBranchName(LONG_B, 18));
});

test('省略後の長さは上限を超えない', () => {
  for (const limit of [3, 5, 8, 12, 18, 40]) {
    const out = abbreviateBranchName(LONG_A, limit);
    assert.ok(Array.from(out).length <= limit, `limit=${limit} で ${out} が上限を超えた`);
  }
});

test('上限を 3 未満にしても、頭と尻が 1 文字ずつ残る', () => {
  assert.equal(abbreviateBranchName('abcdef', 1), 'a…f');
  assert.equal(abbreviateBranchName('abcdef', 0), 'a…f');
  assert.equal(abbreviateBranchName('abcdef', -5), 'a…f');
});

test('空・null・undefined は空文字を返す（ブランチの部分ごと出さないため）', () => {
  assert.equal(abbreviateBranchName(''), '');
  assert.equal(abbreviateBranchName(null), '');
  assert.equal(abbreviateBranchName(undefined), '');
});

test('サロゲートペアを途中で割らない', () => {
  // 絵文字は 1 文字が 2 code unit。slice をそのまま使うと壊れた文字が残る。
  const branch = '🍎🍊🍇🍓🍒🍑🍍🍌';
  const out = abbreviateBranchName(branch, 5);
  assert.equal(Array.from(out).length, 5);
  assert.ok(!out.includes('�'), '壊れた文字が残っていない');
  assert.ok(branch.startsWith(out.split('…')[0]));
  assert.ok(branch.endsWith(out.split('…')[1]));
});

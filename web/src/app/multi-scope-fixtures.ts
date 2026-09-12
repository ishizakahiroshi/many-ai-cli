// multi-scope-fixtures.ts — multi タブの範囲（この箱だけ／全部）の不変条件を固定する。
//
// ここが赤くなったら、直すのはテストではなく実装。とくに次の 2 つは利用者のデータが
// 壊れる側の失敗なので、テストを緩めて通してはいけない。
//
//  1. 範囲を 'box' にしても、保存される並び順（order）から他の箱のセッションが落ちない。
//     落ちると「全部」へ戻したときに、手で並べ替えた順序が失われる。
//  2. ペインのドラッグ並べ替えは、スロット番号を order の添字として直接使わない。
//     絞り込むと両者はずれ、掴んでいないセッションが動く。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c3_multi-scope.md

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  MULTI_SCOPE_ALL,
  MULTI_SCOPE_BOX,
  computeRenderPlan,
  effectiveScope,
  normalizeScope,
  orderIndexForSlot,
  overflowCount,
  rebuildOrder,
  reorderForScope,
  visibleIdsFor,
} from './multi-scope.js';

// 合成データ。実在のパス・リポジトリ名は書かない（公開ファイルの層 1 防御）。
const BOX_A = '/src/box-alpha';
const BOX_B = '/src/box-bravo';

// セッション ID → 箱。1/3/5 が箱 A、2/4/6 が箱 B。
const KEYS = new Map<number, string>([
  [1, BOX_A], [2, BOX_B], [3, BOX_A], [4, BOX_B], [5, BOX_A], [6, BOX_B],
]);
const keyOf = (id: number) => KEYS.get(id) ?? null;

/** 1 回ぶんの render。multi-pane.ts の render() が呼ぶのと同じ関数を同じ順で通す。 */
function renderOnce(prevOrder: number[], scope: string, openKey: string | null, live: number[], capacity = 4) {
  return computeRenderPlan({
    prevOrder,
    liveSortedIds: live,
    scope,
    openProjectKey: openKey,
    projectKeyOf: keyOf,
    capacity,
  });
}

// ---- 範囲の正規化 -----------------------------------------------------------

test('知らない保存値は「全部」へ倒れる', () => {
  assert.equal(normalizeScope('box'), MULTI_SCOPE_BOX);
  assert.equal(normalizeScope('all'), MULTI_SCOPE_ALL);
  assert.equal(normalizeScope(''), MULTI_SCOPE_ALL);
  assert.equal(normalizeScope(null), MULTI_SCOPE_ALL);
  assert.equal(normalizeScope(undefined), MULTI_SCOPE_ALL);
  assert.equal(normalizeScope('BOX'), MULTI_SCOPE_ALL);
  assert.equal(normalizeScope({ scope: 'box' }), MULTI_SCOPE_ALL);
});

test('どの箱も開いていなければ「この箱だけ」は「全部」として効く', () => {
  assert.equal(effectiveScope('box', null), MULTI_SCOPE_ALL);
  assert.equal(effectiveScope('box', ''), MULTI_SCOPE_ALL);
  assert.equal(effectiveScope('box', BOX_A), MULTI_SCOPE_BOX);
  assert.equal(effectiveScope('all', BOX_A), MULTI_SCOPE_ALL);
});

// ---- 並び順の再構築 ---------------------------------------------------------

test('並び順は生存ぶんを保持し、新規を sorted 順で末尾へ足す', () => {
  assert.deepEqual(rebuildOrder([3, 1], [1, 2, 3, 4]), [3, 1, 2, 4]);
});

test('終了したセッションは並び順から落ちる', () => {
  assert.deepEqual(rebuildOrder([3, 1, 2], [1, 3]), [3, 1]);
});

// ---- 罠 1: 範囲を切り替えても保存される並び順が削られない ------------------

test('「この箱だけ」にしても、保存する並び順には他の箱のセッションが残る', () => {
  const live = [1, 2, 3, 4];
  const plan = renderOnce([1, 2, 3, 4], 'box', BOX_A, live);
  assert.deepEqual(plan.order, [1, 2, 3, 4], 'order は全件のまま');
  assert.deepEqual(plan.visibleIds, [1, 3], '表示は箱 A だけ');
});

test('手で並べ替えてから「この箱だけ」→「全部」と往復しても、並び順が元のまま', () => {
  const live = [1, 2, 3, 4];
  // 利用者が手で並べ替えた状態（sorted 順 1,2,3,4 とは違う）。
  const reordered = [4, 1, 3, 2];

  // multi タブを「この箱だけ」で開く（= render が 1 回走る）。
  const scoped = renderOnce(reordered, 'box', BOX_A, live);
  assert.deepEqual(scoped.visibleIds, [1, 3]);
  assert.deepEqual(scoped.order, reordered, '絞り込んだ render で order が削られない');

  // そのまま何度描いても削れない（1Hz の再描画で静かに削られる形の事故を塞ぐ）。
  const again = renderOnce(scoped.order, 'box', BOX_A, live);
  assert.deepEqual(again.order, reordered);

  // 「全部」へ戻す。
  const back = renderOnce(again.order, 'all', BOX_A, live);
  assert.deepEqual(back.order, reordered, '往復しても手で並べ替えた順序が残る');
  assert.deepEqual(back.visibleIds, reordered, '全部では order がそのまま表示順になる');
});

test('「この箱だけ」で開いている間に別の箱でセッションが増えても、並び順へ記録される', () => {
  const plan = renderOnce([1, 2, 3], 'box', BOX_A, [1, 2, 3, 6]);
  assert.deepEqual(plan.order, [1, 2, 3, 6], '見えていない新規も order には入る');
  assert.deepEqual(plan.visibleIds, [1, 3], '見えるのは箱 A だけ');
});

test('どの箱も開いていなければ、全部が並ぶ', () => {
  const plan = renderOnce([1, 2, 3, 4], 'box', null, [1, 2, 3, 4]);
  assert.equal(plan.effectiveScope, MULTI_SCOPE_ALL);
  assert.deepEqual(plan.visibleIds, [1, 2, 3, 4]);
});

test('「全部」のときの visibleIds は order と同じ（従来の挙動）', () => {
  const plan = renderOnce([3, 1, 4, 2], 'all', BOX_A, [1, 2, 3, 4]);
  assert.deepEqual(plan.visibleIds, plan.order);
});

test('visibleIdsFor は order を書き換えない', () => {
  const order = [1, 2, 3, 4];
  const visible = visibleIdsFor(order, 'box', BOX_B, keyOf);
  assert.deepEqual(visible, [2, 4]);
  assert.deepEqual(order, [1, 2, 3, 4]);
  visible.push(99);
  assert.deepEqual(order, [1, 2, 3, 4], '戻り値はコピー');
});

// ---- スロットとあふれ -------------------------------------------------------

test('スロットは visibleIds の先頭から埋まり、余りは空になる', () => {
  const plan = renderOnce([1, 2, 3, 4], 'box', BOX_A, [1, 2, 3, 4], 4);
  assert.deepEqual(plan.slotIds, [1, 3, null, null]);
  assert.equal(plan.overflow, 0);
});

test('枠に入りきらない件数が「他 N 件」になる', () => {
  const plan = renderOnce([1, 2, 3, 4, 5, 6], 'all', BOX_A, [1, 2, 3, 4, 5, 6], 4);
  assert.deepEqual(plan.slotIds, [1, 2, 3, 4]);
  assert.equal(plan.overflow, 2);
});

test('あふれの数え方は範囲で絞った後の件数で決まる', () => {
  const plan = renderOnce([1, 2, 3, 4, 5, 6], 'box', BOX_A, [1, 2, 3, 4, 5, 6], 2);
  assert.deepEqual(plan.visibleIds, [1, 3, 5]);
  assert.equal(plan.overflow, 1);
});

test('overflowCount は負にならない', () => {
  assert.equal(overflowCount(2, 4), 0);
  assert.equal(overflowCount(0, 0), 0);
  assert.equal(overflowCount(5, 4), 1);
  assert.equal(overflowCount(Number.NaN, 4), 0);
});

// ---- 罠 2: スロット番号を order の添字として直接使わない --------------------

test('「全部」ならスロット番号と order の添字は一致する（従来どおり）', () => {
  const order = [7, 8, 9];
  assert.equal(orderIndexForSlot(order, order, 0), 0);
  assert.equal(orderIndexForSlot(order, order, 2), 2);
  assert.equal(orderIndexForSlot(order, order, 3), -1, '空スロットは -1');
  assert.equal(orderIndexForSlot(order, order, -1), -1);
});

test('「この箱だけ」ではスロット番号が order の添字とずれる', () => {
  const order = [1, 2, 3, 4];
  const visible = [1, 3];
  assert.equal(orderIndexForSlot(order, visible, 1), 2, 'スロット 1 は order の 2 番目');
});

test('「この箱だけ」で入れ替えても、他の箱のセッションは動かない', () => {
  const order = [1, 2, 3, 4, 5, 6];
  const visible = visibleIdsFor(order, 'box', BOX_A, keyOf); // [1, 3, 5]
  // スロット 0（#1）とスロット 2（#5）を入れ替える。
  const next = reorderForScope(order, visible, 0, 2);
  assert.deepEqual(next, [5, 2, 3, 4, 1, 6]);
  // 箱 B の並び（2, 4, 6）は相対順も位置も変わっていない。
  assert.deepEqual(next.filter(id => keyOf(id) === BOX_B), [2, 4, 6]);
  assert.deepEqual(
    next.map((id, i) => (keyOf(id) === BOX_B ? i : -1)).filter(i => i >= 0),
    order.map((id, i) => (keyOf(id) === BOX_B ? i : -1)).filter(i => i >= 0),
  );
});

test('「全部」の入れ替えは従来の swap と同じ結果になる', () => {
  const order = [1, 2, 3, 4];
  assert.deepEqual(reorderForScope(order, order, 0, 3), [4, 2, 3, 1]);
  assert.deepEqual(order, [1, 2, 3, 4], '入力を書き換えない');
});

test('「全部」で空スロットへ落とすと末尾へ送られる（従来どおり）', () => {
  const order = [1, 2, 3];
  assert.deepEqual(reorderForScope(order, order, 0, 5), [2, 3, 1]);
});

test('「この箱だけ」で空スロットへ落とすと、見えている列の最後の直後へ入る', () => {
  const order = [1, 2, 3, 4, 5, 6];
  const visible = visibleIdsFor(order, 'box', BOX_A, keyOf); // [1, 3, 5]
  const next = reorderForScope(order, visible, 0, 9);
  // #1 は #5（見えている列の最後）の直後へ。#6 より後ろへは行かない。
  assert.deepEqual(next, [2, 3, 4, 5, 1, 6]);
  assert.deepEqual(visibleIdsFor(next, 'box', BOX_A, keyOf), [3, 5, 1]);
  assert.deepEqual(next.filter(id => keyOf(id) === BOX_B), [2, 4, 6], '他の箱の順序は不変');
});

test('見えているのが 1 件だけのとき、空スロットへ落としても並びは変わらない', () => {
  const order = [1, 2, 4, 6];
  const visible = visibleIdsFor(order, 'box', BOX_A, keyOf); // [1]
  assert.deepEqual(reorderForScope(order, visible, 0, 3), [1, 2, 4, 6]);
});

test('掴んだスロットが空なら何もしない', () => {
  const order = [1, 2, 3];
  assert.deepEqual(reorderForScope(order, [1, 3], 5, 0), [1, 2, 3]);
  assert.deepEqual(reorderForScope(order, [1, 3], 0, 0), [1, 2, 3]);
});

// multi-scope.ts — multi タブ（複数ペイン）の「範囲」の純関数。
//
// DOM にも localStorage にも触らない（node:test から検証するため。sidebar-tree.ts /
// session-strip-branch.ts と同じ切り分け）。描画と永続化は multi-pane.ts が担当する。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c3_multi-scope.md
//
// ここが守っている不変条件は 1 つだけ。壊すと利用者の並べ替えが黙って消える。
//
//   **並び順（order）は常に全セッションの並びで、範囲で絞ってはいけない。**
//
// multi-pane.ts の render() は order を「生存しているセッションだけ」に絞り直して
// localStorage へ保存する。ここへ範囲の絞り込みを混ぜると、他の箱のセッション ID が
// order から落ちたまま保存され、「全部」へ戻したときに利用者が手で並べ替えた順序が
// 失われる。しかも multi タブを 1 回開くだけで起きる（明示的な操作が要らない）ので
// 原因に気づけない。だから「保存する列（order）」と「表示に使う列（visibleIds）」を
// computeRenderPlan が 1 か所で作り分け、後者は決して保存しない。

/** 範囲。'all' = 全セッション（既定・従来の挙動）、'box' = いま開いている箱だけ。 */
export type MultiPaneScope = 'all' | 'box';

export const MULTI_SCOPE_ALL: MultiPaneScope = 'all';
export const MULTI_SCOPE_BOX: MultiPaneScope = 'box';

/** 範囲の保存先。行列数（multiPaneCols / multiPaneRows）と同じ端末ごとの設定。 */
export const STORAGE_MULTI_SCOPE_KEY = 'multiPaneScope';

/** 保存値・入力値を正規化する。知らない値は既定の 'all' へ倒す。 */
export function normalizeScope(raw: unknown): MultiPaneScope {
  return String(raw ?? '') === MULTI_SCOPE_BOX ? MULTI_SCOPE_BOX : MULTI_SCOPE_ALL;
}

/**
 * 実際に効く範囲。
 *
 * どの箱も開いていないときに 'box' が選ばれていたら 'all' として扱う（保存値は変えない）。
 * 起動直後はどの箱も開いていない状態からはじまるので、この分岐が無いと multi が空になる。
 */
export function effectiveScope(scope: unknown, openProjectKey: unknown): MultiPaneScope {
  const normalized = normalizeScope(scope);
  if (normalized === MULTI_SCOPE_BOX && !openProjectKey) return MULTI_SCOPE_ALL;
  return normalized;
}

/**
 * 並び順を生存セッションで作り直す。
 *
 * 既存順のうち生存しているものを保持し、未登録の新規を sorted 順で末尾へ足す
 * （multi-pane.ts が以前インラインで持っていた処理そのまま）。
 *
 * **liveSortedIds には範囲で絞る前の全件を渡すこと。** 絞った列を渡すと、絞られた側の
 * ID が戻り値から落ちる＝それがそのまま保存される。
 */
export function rebuildOrder(prevOrder: readonly number[], liveSortedIds: readonly number[]): number[] {
  const live = new Set(liveSortedIds);
  const next = (Array.isArray(prevOrder) ? prevOrder : []).filter(id => live.has(id));
  const seen = new Set(next);
  for (const id of liveSortedIds) {
    if (!seen.has(id)) { next.push(id); seen.add(id); }
  }
  return next;
}

/**
 * 表示に使う ID の列。**戻り値を保存しない。**
 *
 * 'all' なら order のコピー、'box' なら order の順のまま開いている箱のものだけを残す。
 */
export function visibleIdsFor(
  order: readonly number[],
  scope: unknown,
  openProjectKey: string | null | undefined,
  projectKeyOf: (id: number) => string | null | undefined,
): number[] {
  const list = Array.isArray(order) ? order.slice() : [];
  if (effectiveScope(scope, openProjectKey) === MULTI_SCOPE_ALL) return list;
  return list.filter(id => projectKeyOf(id) === openProjectKey);
}

/**
 * スロット番号 → order の添字。**この変換を通さずにスロット番号を order の添字として
 * 使わない**（範囲で絞ると両者は一致しない）。該当が無ければ -1。
 */
export function orderIndexForSlot(
  order: readonly number[],
  visibleIds: readonly number[],
  slot: number,
): number {
  if (!Number.isInteger(slot) || slot < 0 || slot >= visibleIds.length) return -1;
  return order.indexOf(visibleIds[slot]);
}

/** 枠（cols × rows）に入りきらなかった件数。0 のときは「他 N 件」を出さない。 */
export function overflowCount(visibleCount: number, capacity: number): number {
  const visible = Number.isFinite(visibleCount) ? Math.max(0, Math.floor(visibleCount)) : 0;
  const cap = Number.isFinite(capacity) ? Math.max(0, Math.floor(capacity)) : 0;
  return Math.max(0, visible - cap);
}

export interface RenderPlanInput {
  /** いまの並び順（全セッション基準）。 */
  prevOrder: readonly number[];
  /** 生存しているセッションの ID を sorted 順で。**範囲で絞る前の全件**を渡す。 */
  liveSortedIds: readonly number[];
  /** 利用者が選んでいる範囲（保存値そのまま）。 */
  scope: unknown;
  /** いま開いている箱のキー。null ならどの箱も開いていない。 */
  openProjectKey: string | null | undefined;
  /** セッション ID → 箱のキー。'box' のときだけ呼ばれる（'all' では 1 度も呼ばれない）。 */
  projectKeyOf: (id: number) => string | null | undefined;
  /** 枠の数（cols × rows）。 */
  capacity: number;
}

export interface RenderPlan {
  /** 保存する並び。常に全セッションぶん。 */
  order: number[];
  /** 表示に使う列。保存しない。 */
  visibleIds: number[];
  /** スロットに入れる ID。長さは capacity。空きは null。 */
  slotIds: (number | null)[];
  /** 枠に入りきらなかった件数。 */
  overflow: number;
  /** 実際に効いた範囲。 */
  effectiveScope: MultiPaneScope;
}

/**
 * 1 回の render で要る 4 つ（保存する並び / 表示する列 / スロット / あふれた件数）を作る。
 *
 * multi-pane.ts の render() はこの戻り値をそのまま使う＝絞り込みと並び順の再構築が
 * 2 か所に散らない。テスト（multi-scope-fixtures.ts）もこの関数を直接叩く。
 */
export function computeRenderPlan(input: RenderPlanInput): RenderPlan {
  const order = rebuildOrder(input.prevOrder, input.liveSortedIds);
  const eff = effectiveScope(input.scope, input.openProjectKey);
  const visibleIds = eff === MULTI_SCOPE_ALL
    ? order.slice()
    : order.filter(id => input.projectKeyOf(id) === input.openProjectKey);
  const capacity = Number.isFinite(input.capacity) ? Math.max(0, Math.floor(input.capacity)) : 0;
  const slotIds: (number | null)[] = [];
  for (let i = 0; i < capacity; i++) {
    slotIds.push(i < visibleIds.length ? visibleIds[i] : null);
  }
  return {
    order,
    visibleIds,
    slotIds,
    overflow: overflowCount(visibleIds.length, capacity),
    effectiveScope: eff,
  };
}

/**
 * ペインをドラッグで入れ替えたあとの並び順。**order は全件のまま返る。**
 *
 * - 落とした先が埋まっている: その 2 つを order の中で入れ替える（他の箱の要素は動かない）
 * - 落とした先が空スロット: 掴んだものを「表示されている列の最後」の直後へ動かす
 *
 * 範囲が 'all' のとき（visibleIds === order）は、どちらも従来の swap / 末尾送りと
 * 同じ結果になる。
 */
export function reorderForScope(
  order: readonly number[],
  visibleIds: readonly number[],
  fromSlot: number,
  toSlot: number,
): number[] {
  const next = Array.isArray(order) ? order.slice() : [];
  const fromIdx = orderIndexForSlot(next, visibleIds, fromSlot);
  if (fromIdx < 0 || fromSlot === toSlot) return next;
  const fromId = next[fromIdx];

  const toIdx = orderIndexForSlot(next, visibleIds, toSlot);
  if (toIdx >= 0) {
    next[fromIdx] = next[toIdx];
    next[toIdx] = fromId;
    return next;
  }

  // 空スロットへ落とした: 表示されている列の末尾へ送る。
  const moved = next.slice();
  moved.splice(fromIdx, 1);
  let anchor = -1;
  for (let i = visibleIds.length - 1; i >= 0; i--) {
    const id = visibleIds[i];
    if (id === fromId) continue;
    const idx = moved.indexOf(id);
    if (idx >= 0) { anchor = idx; break; }
  }
  // 掴んだもの以外に表示されているものが無い＝送る先が無い。並びを触らない
  // （ここで末尾へ push すると、画面では何も起きていないのに「全部」の並びだけが動く）。
  if (anchor < 0) return next;
  moved.splice(anchor + 1, 0, fromId);
  return moved;
}

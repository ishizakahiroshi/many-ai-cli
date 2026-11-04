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

/**
 * multi を開いたときの範囲。'ask' = 開くたびに選ばせる（既定）。
 * 由来: docs/local/plan_multi-repo-scope.md（2026-11-04 利用者の決定）
 */
export type MultiPaneScopeDefault = 'ask' | MultiPaneScope;
export const MULTI_SCOPE_DEFAULT_ASK: MultiPaneScopeDefault = 'ask';
export const STORAGE_MULTI_SCOPE_DEFAULT_KEY = 'multiPaneScopeDefault';
/** ピン留めしたセッション（`id:started_at` の配列）。端末ごと。 */
export const STORAGE_MULTI_PINNED_KEY = 'multiPanePinned';
/** 入りきらないとき枠を自動で増やすか（'1' / '0'）。端末ごと・既定オフ。 */
export const STORAGE_MULTI_AUTO_GROW_KEY = 'multiPaneAutoGrow';
/** ピンの保存件数の上限。古いものから捨てる（切断中に消すと記憶が飛ぶので自動では掃除しない）。 */
export const MULTI_PINNED_MAX = 200;
/** 自動で広げるときの上限。レイアウト選択（GridPicker）の 6×3 と同じ。 */
export const MULTI_GRID_MAX_COLS = 6;
export const MULTI_GRID_MAX_ROWS = 3;

/** 保存値・入力値を正規化する。知らない値は既定の 'all' へ倒す。 */
export function normalizeScope(raw: unknown): MultiPaneScope {
  return String(raw ?? '') === MULTI_SCOPE_BOX ? MULTI_SCOPE_BOX : MULTI_SCOPE_ALL;
}

/** 開いたときの範囲の保存値を正規化する。知らない値・未設定は 'ask'（開くたびに聞く）。 */
export function normalizeScopeDefault(raw: unknown): MultiPaneScopeDefault {
  const value = String(raw ?? '');
  if (value === MULTI_SCOPE_BOX || value === MULTI_SCOPE_ALL) return value;
  return MULTI_SCOPE_DEFAULT_ASK;
}

/**
 * multi を開いたときに使う範囲と、利用者に聞くかどうか。
 *
 * - 既定が 'box' / 'all' なら、それを範囲にする（前回の切り替えは持ち越さない）
 * - 既定が 'ask' なら範囲は今のまま、リポが 1 つ以上あるときだけ聞く
 *   （リポが無いと「このプロジェクトだけ」を選べないので、聞いても選べない）
 */
export function scopeOnOpen(
  defaultValue: unknown,
  currentScope: unknown,
  hasRepo: boolean,
): { scope: MultiPaneScope; ask: boolean } {
  const def = normalizeScopeDefault(defaultValue);
  if (def === MULTI_SCOPE_DEFAULT_ASK) return { scope: normalizeScope(currentScope), ask: !!hasRepo };
  return { scope: normalizeScope(def), ask: false };
}

/**
 * ピンの鍵。Hub を再起動すると ID が振り直されるので、起動時刻と組にして持つ
 * （同じ ID の別セッションへピンが付き直さない）。
 */
export function pinKey(id: unknown, startedAt: unknown): string {
  return `${String(id ?? '')}:${String(startedAt ?? '')}`;
}

/** 保存値を正規化する。文字列以外・重複を捨て、新しいほうから上限件数だけ残す。 */
export function normalizePinnedList(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of raw) {
    if (typeof item !== 'string' || !item || seen.has(item)) continue;
    seen.add(item);
    out.push(item);
  }
  return out.length > MULTI_PINNED_MAX ? out.slice(out.length - MULTI_PINNED_MAX) : out;
}

/** ピン留めを先に、それ以外を後に並べる。各群の中は入力の順のまま。入力は書き換えない。 */
export function prioritizePinned(ids: readonly number[], isPinned?: (id: number) => boolean): number[] {
  const list = Array.isArray(ids) ? ids.slice() : [];
  if (typeof isPinned !== 'function') return list;
  const pinned: number[] = [];
  const rest: number[] = [];
  for (const id of list) (isPinned(id) ? pinned : rest).push(id);
  return pinned.concat(rest);
}

/**
 * 入りきらないとき、件数が入るまで枠を広げた寸法。列 → 行の順に 1 つずつ足す
 * （横長の画面で 1 枚が縦に潰れにくい）。上限は 6×3。件数が収まっていれば元の寸法。
 */
export function autoGrowGrid(cols: number, rows: number, count: number): { cols: number; rows: number } {
  let c = Math.max(1, Math.min(MULTI_GRID_MAX_COLS, Math.floor(Number(cols) || 1)));
  let r = Math.max(1, Math.min(MULTI_GRID_MAX_ROWS, Math.floor(Number(rows) || 1)));
  const need = Math.max(0, Math.floor(Number(count) || 0));
  let growCol = true;
  while (c * r < need && (c < MULTI_GRID_MAX_COLS || r < MULTI_GRID_MAX_ROWS)) {
    if ((growCol && c < MULTI_GRID_MAX_COLS) || r >= MULTI_GRID_MAX_ROWS) c++;
    else r++;
    growCol = !growCol;
  }
  return { cols: c, rows: r };
}

/**
 * 入りきらなかったセッションを、フォーカス中のペインと入れ替えた並び順。
 * **order は全件のまま返る**（範囲の不変条件）。
 *
 * 入れ替え先は、フォーカス中のペインにピンが無ければそこ。ピンがあれば、ピンの無い
 * 最後のペイン。どのペインもピン留めなら入れ替えない（reason='all-pinned'）。
 * ピン留めのセッションは表示の列で常に先に来るので、ピンの無いもの同士を order の中で
 * 入れ替えれば、入れ替えた側がちょうどそのペインに入る。
 */
export function swapIntoSlot(
  order: readonly number[],
  visibleIds: readonly number[],
  capacity: number,
  focusedSlot: number,
  incomingId: number,
  isPinned?: (id: number) => boolean,
): { order: number[]; slot: number; reason: '' | 'not-overflow' | 'all-pinned' } {
  const next = Array.isArray(order) ? order.slice() : [];
  const visible = Array.isArray(visibleIds) ? visibleIds : [];
  const cap = Math.max(0, Math.floor(Number(capacity) || 0));
  const incomingIdx = visible.indexOf(incomingId);
  if (incomingIdx < cap || incomingIdx < 0) return { order: next, slot: -1, reason: 'not-overflow' };
  const pinned = (id: number) => (typeof isPinned === 'function' ? isPinned(id) : false);
  const shown = visible.slice(0, cap);
  let slot = Number.isInteger(focusedSlot) && focusedSlot >= 0 && focusedSlot < shown.length && !pinned(shown[focusedSlot])
    ? focusedSlot : -1;
  if (slot < 0) {
    for (let i = shown.length - 1; i >= 0; i--) {
      if (!pinned(shown[i])) { slot = i; break; }
    }
  }
  if (slot < 0) return { order: next, slot: -1, reason: 'all-pinned' };
  const outgoing = shown[slot];
  const a = next.indexOf(outgoing);
  const b = next.indexOf(incomingId);
  if (a < 0 || b < 0) return { order: next, slot: -1, reason: 'not-overflow' };
  next[a] = incomingId;
  next[b] = outgoing;
  return { order: next, slot, reason: '' };
}

/** 上の段に並べるリポ 1 件。 */
export interface RepoChip {
  key: string;
  count: number;
}

/**
 * リポ名の並び。入力（サイドバーと同じ順に並んだセッション）で最初に出た順に、
 * セッション数を数える。キーが空のものは数えない。
 */
export function repoChipsFor(entries: Iterable<{ key: string | null | undefined }>): RepoChip[] {
  const out: RepoChip[] = [];
  const byKey = new Map<string, RepoChip>();
  for (const entry of entries || []) {
    const key = entry && entry.key ? String(entry.key) : '';
    if (!key) continue;
    let chip = byKey.get(key);
    if (!chip) {
      chip = { key, count: 0 };
      byKey.set(key, chip);
      out.push(chip);
    }
    chip.count++;
  }
  return out;
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
  /** ピン留めか。渡すと表示の列でピン留めが先に来る（order は変えない）。 */
  isPinned?: (id: number) => boolean;
}

export interface RenderPlan {
  /** 保存する並び。常に全セッションぶん。 */
  order: number[];
  /** 表示に使う列（ピン留めが先）。保存しない。 */
  visibleIds: number[];
  /** スロットに入れる ID。長さは capacity。空きは null。 */
  slotIds: (number | null)[];
  /** 枠に入りきらなかった件数。 */
  overflow: number;
  /** 枠に入りきらなかったセッション（表示の列の順）。 */
  overflowIds: number[];
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
  const scoped = eff === MULTI_SCOPE_ALL
    ? order.slice()
    : order.filter(id => input.projectKeyOf(id) === input.openProjectKey);
  const visibleIds = prioritizePinned(scoped, input.isPinned);
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
    overflowIds: visibleIds.slice(capacity),
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

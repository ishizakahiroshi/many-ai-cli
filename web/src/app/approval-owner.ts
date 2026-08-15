// action-bar の所有者判定。
//
// #action-bar は全セッション共有の DOM が 1 個だけで、描画時に
// dataset.approvalSessionId へ描画元のセッション番号が焼かれる（approval.ts の
// showActionBar）。「いま出ているパネルは誰のものか」を問う場所は 2 つある。
//
//   1. セッション切替時 — 切替先のものでなければ捨てる
//   2. 再描画要否の判定 — 所有者が違うなら描き直す
//
// 式を二重に持つと片方だけ更新されて取り違えが復活するので、ここへ集約する。
// approval.ts に置かないのは、あのファイルが i18n / ws-client / terminal まで
// 芋づるで引き込み、node --test の fixture から import できないため。
//
// 正本: bugfix_approval-panel-shows-other-session_2026-08-14.md

/** action-bar として最小限必要な形。テストから Fake を渡せるよう構造的に定義する。 */
export interface ActionBarLike {
  dataset: Record<string, string | undefined>;
  classList: { contains(token: string): boolean; remove(...tokens: string[]): void };
  children: { length: number };
  innerHTML: string;
}

type MaybeBar = ActionBarLike | null | undefined;

/** 描画元のセッション番号。焼かれていなければ null。 */
export function actionBarOwner(bar: MaybeBar): string | null {
  const owner = bar && bar.dataset ? bar.dataset.approvalSessionId : undefined;
  return owner === undefined || owner === '' ? null : String(owner);
}

/**
 * 出ているパネルが id 以外のセッションのものなら true。
 * 所有者が焼かれていない場合は false を返す（誰のものか分からないものを
 * 「他人のもの」と決めつけない。空の bar を毎回掃除しても意味がない）。
 */
export function actionBarOwnedByOther(bar: MaybeBar, id: unknown): boolean {
  const owner = actionBarOwner(bar);
  return owner !== null && owner !== String(id);
}

/**
 * id のパネルを描き直す必要があるか。
 * bar が無い・非表示・中身が空・所有者違いのいずれか。
 *
 * 所有者違いを条件に含めるのが 2026-08-14 の修正点。それまでは見た目
 * （visible / children）だけで決めていたため、別セッションのパネルが出たまま
 * 切り替えると「visible で children もある」→ 描き直さない、となって
 * 切替元のパネルが残っていた。
 */
export function actionBarNeedsRepaint(bar: MaybeBar, id: unknown): boolean {
  if (!bar) return true;
  if (!bar.classList.contains('visible')) return true;
  if (bar.children.length === 0) return true;
  return actionBarOwnedByOther(bar, id);
}

/**
 * bar の DOM と所有者マークだけを捨てる。
 *
 * 承認状態（approvalVisibleCache / approvalRawOptionsCache / Hub への
 * session_hint）には触らない。切替元の承認はまだ未回答で、戻ったときに
 * 出し直す必要があるため。承認状態ごと落とすのは hideActionBar の仕事。
 */
export function releaseActionBarOwnership(bar: MaybeBar): void {
  if (!bar) return;
  bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs');
  bar.innerHTML = '';
  delete bar.dataset.approvalSessionId;
  delete bar.dataset.approvalCandidateKey;
  delete bar.dataset.approvalSourceEpoch;
}

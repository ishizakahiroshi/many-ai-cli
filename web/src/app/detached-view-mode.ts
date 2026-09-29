// detached-view-mode.ts — 別窓タブモード（/?view=detached-tab&tab=<名前>）の判定だけを持つ葉モジュール。
// plan: docs/local/plan_detached-tab-windows.md C2
//
// session-list.ts / settings.ts / memo-panel.ts など循環 import の輪の中にいるモジュールから
// 参照されるので、ここは何も import しない（CLAUDE.md 索引「画面の JS を読み込んだ瞬間に…」）。
// URL は読み込み時ではなく、最初に呼ばれた時点で 1 回だけ読む。

export type DetachedTabName =
  | 'chat' | 'files' | 'git' | 'review'
  | 'history' | 'approval' | 'orchestration' | 'multi'
  | 'memo';

export const DETACHED_TAB_NAMES: readonly DetachedTabName[] = [
  'chat', 'files', 'git', 'review', 'history', 'approval', 'orchestration', 'multi', 'memo',
];

/** 表示中のセッションで中身が変わるタブ（追従 / 固定の切り替えを出すもの）。 */
export const SESSION_BOUND_DETACHED_TABS: ReadonlySet<DetachedTabName> = new Set<DetachedTabName>([
  'chat', 'files', 'git', 'review', 'memo',
]);

export interface DetachedTabParams {
  tab: DetachedTabName;
  /** 開いたときに表示するセッション（無ければ 0） */
  sessionId: number;
}

let parsed: DetachedTabParams | null | undefined;

function parse(): DetachedTabParams | null {
  if (typeof window === 'undefined' || !window.location) return null;
  const params = new URLSearchParams(window.location.search);
  if (params.get('view') !== 'detached-tab') return null;
  const tab = params.get('tab') as DetachedTabName;
  if (!DETACHED_TAB_NAMES.includes(tab)) return null;
  const sessionId = parseInt(params.get('session') || '0', 10);
  return { tab, sessionId: Number.isInteger(sessionId) && sessionId > 0 ? sessionId : 0 };
}

export function detachedTabParams(): DetachedTabParams | null {
  if (parsed === undefined) parsed = parse();
  return parsed;
}

export function isDetachedTabView(): boolean {
  return detachedTabParams() !== null;
}

/** 別窓タブモードで固定しているタブ。別窓でなければ null。 */
export function detachedTabName(): DetachedTabName | null {
  return detachedTabParams()?.tab ?? null;
}

/**
 * Hub の本体の窓か（`view` クエリが無い）。選択中セッションを別窓へ流すのは本体の窓だけ。
 * 別窓グリッド（view=detached-grid）も本体ではない。
 */
export function isMainHubWindow(): boolean {
  if (typeof window === 'undefined' || !window.location) return false;
  return !new URLSearchParams(window.location.search).get('view');
}

// ─── 別窓の中で activateSession() を受ける口 ─────────────────────────────────
// 別窓ではアプリ内部からの activateSession()（初回の自動選択・承認の自動移動・
// 削除後の付け替え）を本体と同じ処理で走らせない（PTY の大きさの主導権を Hub へ
// 名乗り出てしまう）。session-list.ts はここへ渡すだけで、何をするかは detached-view.ts が決める。

type ActivateHandler = (id: number) => void;
let activateHandler: ActivateHandler | null = null;

export function setDetachedActivateHandler(fn: ActivateHandler): void {
  activateHandler = fn;
}

export function forwardDetachedActivate(id: number): void {
  activateHandler?.(id);
}

// ─── 別窓を開く ─────────────────────────────────────────────────────────────

export function buildDetachedTabUrl(tab: DetachedTabName, sessionId?: number | null): string {
  const sid = sessionId && sessionId > 0 ? `&session=${sessionId}` : '';
  return `/?view=detached-tab&tab=${encodeURIComponent(tab)}${sid}`;
}

/**
 * 同じタブは同じ名前の窓を使う。2 回目は新しい窓を増やさず、既存の窓を読み直さずに前へ出す
 * （URL を渡して window.open すると、既存の窓がその URL で読み込み直される）。
 */
export function openDetachedTabWindow(tab: DetachedTabName, sessionId?: number | null): void {
  const url = buildDetachedTabUrl(tab, sessionId);
  const win = window.open('', `many-ai-cli-detached-${tab}`);
  if (!win) return; // ポップアップが止められた
  let alreadyOpen = false;
  try {
    const current = win.location.href;
    alreadyOpen = !!current && current !== 'about:blank' && win.location.search.includes('view=detached-tab');
  } catch (_) { /* 同じ origin なので読めるはず。読めなければ開き直す */ }
  if (!alreadyOpen) win.location.href = url;
  try { win.focus(); } catch (_) { /* focus できなくても開いてはいる */ }
}

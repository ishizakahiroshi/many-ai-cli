// detached-view-mode.ts — 別窓タブモード（/?view=detached-tab&tab=<名前>）の判定だけを持つ葉モジュール。
// plan: docs/local/plan_detached-tab-windows.md C2 / docs/local/plan_file-preview-popout-window.md C1
//
// session-list.ts / settings.ts / memo-panel.ts など循環 import の輪の中にいるモジュールから
// 参照されるので、ここは何も import しない（CLAUDE.md 索引「画面の JS を読み込んだ瞬間に…」）。
// URL は読み込み時ではなく、最初に呼ばれた時点で 1 回だけ読む。

// memo と file はタブではないが、同じ別窓の仕組みで 1 枚を全面に出す。
// file はファイル 1 枚のプレビュー（path と cwd を URL で受け取る）。
export type DetachedTabName =
  | 'chat' | 'files' | 'git' | 'review'
  | 'history' | 'approval' | 'orchestration' | 'multi'
  | 'memo' | 'file';

export const DETACHED_TAB_NAMES: readonly DetachedTabName[] = [
  'chat', 'files', 'git', 'review', 'history', 'approval', 'orchestration', 'multi', 'memo', 'file',
];

/** 表示中のセッションで中身が変わるタブ（追従 / 固定の切り替えを出すもの）。 */
export const SESSION_BOUND_DETACHED_TABS: ReadonlySet<DetachedTabName> = new Set<DetachedTabName>([
  'chat', 'files', 'git', 'review', 'memo',
]);

export interface DetachedTabParams {
  tab: DetachedTabName;
  /** 開いたときに表示するセッション（無ければ 0） */
  sessionId: number;
  /** tab=file のときだけ: 表示するファイルの絶対パス（ほかのタブでは空） */
  path: string;
  /** tab=file のときだけ: 相対パス表示とプレビューの起点（無ければ空） */
  cwd: string;
}

/** URL の検索文字列（`?view=…`）を読む。window に触らないので fixture からも呼べる。 */
export function parseDetachedTabSearch(search: string): DetachedTabParams | null {
  const params = new URLSearchParams(search);
  if (params.get('view') !== 'detached-tab') return null;
  const tab = params.get('tab') as DetachedTabName;
  if (!DETACHED_TAB_NAMES.includes(tab)) return null;
  const sessionId = parseInt(params.get('session') || '0', 10);
  const path = tab === 'file' ? (params.get('path') || '') : '';
  const cwd = tab === 'file' ? (params.get('cwd') || '') : '';
  // 出すファイルが分からない窓は、未知のタブ名と同じく別窓として扱わない
  if (tab === 'file' && !path) return null;
  return { tab, sessionId: Number.isInteger(sessionId) && sessionId > 0 ? sessionId : 0, path, cwd };
}

let parsed: DetachedTabParams | null | undefined;

function parse(): DetachedTabParams | null {
  if (typeof window === 'undefined' || !window.location) return null;
  return parseDetachedTabSearch(window.location.search);
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
 * 大きさを渡して、新しいタブではなく独立した窓で開く（サブモニターへそのまま動かせる。
 * plan_file-preview-popout-window.md C3）。窓が取れなかった（ポップアップが止められた）ら false。
 */
export function openDetachedTabWindow(tab: DetachedTabName, sessionId?: number | null): boolean {
  const url = buildDetachedTabUrl(tab, sessionId);
  const win = window.open('', `many-ai-cli-detached-${tab}`, detachedPopupFeatures());
  if (!win) return false;
  let alreadyOpen = false;
  try {
    const current = win.location.href;
    alreadyOpen = !!current && current !== 'about:blank' && win.location.search.includes('view=detached-tab');
  } catch (_) { /* 同じ origin なので読めるはず。読めなければ開き直す */ }
  if (!alreadyOpen) win.location.href = url;
  try { win.focus(); } catch (_) { /* focus できなくても開いてはいる */ }
  return true;
}

// ─── ファイルのプレビューを別窓で開く ───────────────────────────────────────
// plan: docs/local/plan_file-preview-popout-window.md C1
//
// サブモニターへ置いたまま表示しておく用途なので、窓は 1 枚を使い回す。2 回目以降は
// 窓を読み直さず、postMessage で中身だけ差し替える（窓は置いた場所に残る）。

export interface DetachedWindowSize {
  width: number;
  height: number;
}

const DEFAULT_WINDOW_SIZE: DetachedWindowSize = { width: 960, height: 1000 };
const FILE_WINDOW_NAME = 'many-ai-cli-file-viewer';

export const OPEN_FILE_MESSAGE_TYPE = 'many-ai-cli-open-file';

/** 開き元 → ファイルの別窓: 表示するファイルを差し替える */
export interface OpenFileMessage {
  type: typeof OPEN_FILE_MESSAGE_TYPE;
  path: string;
  sessionId: number;
  cwd: string;
}

/**
 * 大きさを渡すと、ブラウザは新しいタブではなく独立した窓（popup）で開く
 * （HTML 仕様の「popup かどうか」の判定。何も渡さない window.open はタブになる）。
 */
export function detachedPopupFeatures(size?: DetachedWindowSize | null): string {
  const s = size && size.width > 0 && size.height > 0 ? size : DEFAULT_WINDOW_SIZE;
  let width = Math.round(s.width);
  let height = Math.round(s.height);
  if (typeof screen !== 'undefined') {
    if (screen.availWidth > 0) width = Math.min(width, screen.availWidth);
    if (screen.availHeight > 0) height = Math.min(height, screen.availHeight);
  }
  return `popup,width=${width},height=${height}`;
}

export function buildDetachedFileUrl(path: string, sessionId?: number | null, cwd?: string): string {
  const q = new URLSearchParams({ view: 'detached-tab', tab: 'file', path });
  if (sessionId && sessionId > 0) q.set('session', String(sessionId));
  if (cwd) q.set('cwd', cwd);
  return `/?${q.toString()}`;
}

/**
 * ファイルのプレビューを別窓で開く。窓が取れなかった（ポップアップが止められた）ら false。
 * トーストは呼び出し側が出す。
 */
export function openDetachedFileWindow(
  path: string,
  sessionId?: number | null,
  cwd?: string,
  size?: DetachedWindowSize | null,
): boolean {
  // 名前付きの窓が既にあれば大きさの指定は無視され、その窓が返る（URL が空なので読み直さない）
  const win = window.open('', FILE_WINDOW_NAME, detachedPopupFeatures(size));
  if (!win) return false;
  let alreadyOpen = false;
  try {
    alreadyOpen = parseDetachedTabSearch(win.location.search)?.tab === 'file';
  } catch (_) { /* 同じ origin なので読めるはず。読めなければ開き直す */ }
  if (alreadyOpen) {
    const msg: OpenFileMessage = {
      type: OPEN_FILE_MESSAGE_TYPE,
      path,
      sessionId: sessionId && sessionId > 0 ? sessionId : 0,
      cwd: cwd || '',
    };
    win.postMessage(msg, window.location.origin);
  } else {
    win.location.href = buildDetachedFileUrl(path, sessionId, cwd);
  }
  try { win.focus(); } catch (_) { /* focus できなくても開いてはいる */ }
  return true;
}

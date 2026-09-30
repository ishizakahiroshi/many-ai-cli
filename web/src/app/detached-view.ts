// detached-view.ts — タブ 1 枚・作業メモ・ファイル 1 枚を別窓で開く（/?view=detached-tab&tab=<名前>）。
// plan: docs/local/plan_detached-tab-windows.md C2〜C5 / docs/local/plan_file-preview-popout-window.md C1
//
// 別窓グリッド（detached-grid.ts）と同じく、アプリ全体を読み込んだうえで要らない部分を隠す。
// 各タブの描画コードには手を入れず、setActiveTab() で 1 枚を全面に出すだけ。
//
// 別窓で本体と同じ処理を走らせない約束（detached-view-mode.ts の判定を各所が見る）:
// - activateSession() はアプリ内部から効かない。別窓のセッション選択は showDetachedSession() だけ
//   （PTY の大きさの主導権・入力欄・承認パネル・スポーン確認に触れない）
// - 通知音とデスクトップ通知は鳴らさない（本体と二重になる）
// - 選択中セッションを流すのは本体の窓だけ（別窓は受け取るだけ。往復のループを作らない）

import { t } from '../i18n.js';
import { showToast } from './util.js';
import { activeSessionId, sessions, set_activeSessionId } from './state.js';
import { setActiveTab, updateChatCountBadge } from './settings.js';
import { activateSession } from './session-list.js';
import { rewireChatHistorySub } from './chat-history.js';
import { openSpawnPanelWith } from './spawn-panel.js';
import {
  basenameForPath,
  dirnameForPath,
  getOrCreatePathPopup,
  mountSingleFilePreview,
  renderPathPopupItems,
} from './path-links.js';
import { providerDisplayName } from './provider-icon.js';
import { mountMemoWindow, refreshMemoContext } from './memo-panel.js';
import {
  DETACHED_TAB_NAMES,
  OPEN_FILE_MESSAGE_TYPE,
  SESSION_BOUND_DETACHED_TABS,
  buildDetachedFileUrl,
  detachedTabParams,
  isMainHubWindow,
  openDetachedTabWindow,
  setDetachedActivateHandler,
} from './detached-view-mode.js';
import type { DetachedTabName, DetachedTabParams, OpenFileMessage } from './detached-view-mode.js';
import { ackWindowRequest, onWindowMessage, postWindowMessage, requestMainWindow } from './window-channel.js';

const FOLLOW_STORAGE_KEY = 'many-ai-cli-detached-follow';

// 別窓グリッドと同じく隠す要素（detached-grid.ts の hideIds と揃える）。
// ヘッダーは CSS（body.detached-tab-mode header）で隠し、代わりに細い見出しを出す。
const HIDE_IDS = [
  'session-list',
  'sidebar-resizer',
  'settings-panel',
  'input-bar-outer',
  'token-statusbar',
  'action-bar',
  'approval-fold-band',
  'multi-question-banner',
  'stale-binary-banner',
  'approval-suppressed-banner',
  'mobile-menu-btn',
  'mobile-spawn-btn',
  'mobile-drawer-backdrop',
  'mobile-keyboard-panel',
  'about-panel',
  'model-picker-overlay',
  'session-strip',
  'unified-tab-bar',
];

function tabTitle(tab: DetachedTabName): string {
  return t(`detached_tab_title_${tab}`);
}

function isFollowing(): boolean {
  try { return localStorage.getItem(FOLLOW_STORAGE_KEY) !== '0'; } catch (_) { return true; }
}

function setFollowing(on: boolean): void {
  try { localStorage.setItem(FOLLOW_STORAGE_KEY, on ? '1' : '0'); } catch (_) { /* 保存できなくても今の窓では効く */ }
}

function sessionLabel(id: number | null): string {
  if (id === null || id === undefined) return t('detached_no_session');
  const s = sessions.get(id);
  if (!s) return t('detached_no_session');
  const name = s.label || basenameForPath(s.cwd || '') || '';
  const provider = providerDisplayName(s.provider) || s.provider || '';
  return [`#${id}`, provider, name].filter(Boolean).join(' ');
}

// ─── 別窓 ─────────────────────────────────────────────────────────────────

let detachedTab: DetachedTabName | null = null;
let initialSessionId = 0;
let pendingFollowId: number | null = null;
let sessionLabelEl: HTMLElement | null = null;

// ファイルの別窓（tab=file）で表示中のファイル。開き元からの postMessage で差し替わる。
interface FileTarget { path: string; sessionId: number; cwd: string }
let fileTarget: FileTarget | null = null;
let fileHost: HTMLElement | null = null;
let fileLabelEl: HTMLElement | null = null;

function updateBar(): void {
  if (detachedTab === 'file') {
    const name = fileTarget ? (basenameForPath(fileTarget.path) || fileTarget.path) : '';
    if (fileLabelEl) {
      fileLabelEl.textContent = name;
      fileLabelEl.title = fileTarget?.path || '';
    }
    // タスクバーで見分けられるよう、ファイル名を先頭に出す
    document.title = `${name} — many-ai-cli`;
    return;
  }
  if (sessionLabelEl) sessionLabelEl.textContent = sessionLabel(activeSessionId);
  if (detachedTab) document.title = `${tabTitle(detachedTab)} — ${sessionLabel(activeSessionId)} — many-ai-cli`;
}

/** 別窓で表示するセッションを変える唯一の入口。activateSession() の本体処理は通さない。 */
function showDetachedSession(id: number): void {
  if (!detachedTab || !sessions.has(id)) return;
  set_activeSessionId(id);
  // ファイルの別窓は開いたファイルに固定。activeSessionId は session-list.ts の render() が
  // 自動選択を繰り返さないように入れるだけ（下のタブ向けの処理は要らない）。
  if (detachedTab === 'file') return;
  try { rewireChatHistorySub(id); } catch (_) { /* チャット以外のタブでは無くてよい */ }
  updateChatCountBadge();
  if (detachedTab === 'memo') refreshMemoContext();
  // セッションに依存しないタブ（履歴・承認・サブエージェント木・マルチ）は初期化時に 1 回出してある。
  // ここで setActiveTab を呼び直すと、マルチでは配置ピッカーが開閉してしまう。
  // activeSessionId だけは入れておく（null のままだと session-list.ts の render() が毎回
  // 自動選択で早期 return し、サブエージェント木やマルチの再描画まで届かない）。
  else if (SESSION_BOUND_DETACHED_TABS.has(detachedTab)) setActiveTab(id, detachedTab);
  updateBar();
}

/**
 * アプリ内部の activateSession()（初回の自動選択・削除後の付け替え・承認の自動移動等）が
 * 別窓で呼ばれたときの受け口。まだ何も表示していないときだけ受ける。
 */
function onAppActivate(id: number): void {
  if (activeSessionId !== null && sessions.has(activeSessionId)) return;
  let next = id;
  if (pendingFollowId !== null && sessions.has(pendingFollowId)) next = pendingFollowId;
  else if (initialSessionId && sessions.has(initialSessionId)) next = initialSessionId;
  pendingFollowId = null;
  initialSessionId = 0;
  showDetachedSession(next);
}

function onFollowMessage(sessionId: number | null): void {
  if (!detachedTab || !SESSION_BOUND_DETACHED_TABS.has(detachedTab) || !isFollowing()) return;
  if (sessionId === null || sessionId === activeSessionId) return;
  if (sessions.has(sessionId)) showDetachedSession(sessionId);
  else pendingFollowId = sessionId; // snapshot 前。届いたら onAppActivate が拾う
}

function buildBar(tab: DetachedTabName): HTMLElement {
  const bar = document.createElement('div');
  bar.id = 'detached-tab-bar';

  const title = document.createElement('strong');
  title.className = 'detached-tab-title';
  title.textContent = tabTitle(tab);
  bar.append(title);

  if (tab === 'file') {
    // セッションの表示と追従 / 固定の切り替えは出さない（ファイルは本体の選択に追従しない）
    fileLabelEl = document.createElement('span');
    fileLabelEl.className = 'detached-tab-session';
    bar.append(fileLabelEl);
  }

  if (SESSION_BOUND_DETACHED_TABS.has(tab)) {
    sessionLabelEl = document.createElement('span');
    sessionLabelEl.className = 'detached-tab-session';
    bar.append(sessionLabelEl);

    const toggle = document.createElement('div');
    toggle.className = 'detached-follow-toggle';
    toggle.setAttribute('role', 'group');
    const mk = (follow: boolean): HTMLButtonElement => {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.textContent = t(follow ? 'detached_follow' : 'detached_fixed');
      btn.title = t(follow ? 'detached_follow_tooltip' : 'detached_fixed_tooltip');
      btn.addEventListener('click', () => {
        setFollowing(follow);
        paint();
        if (follow) postWindowMessage({ type: 'hello' }); // 本体の今の選択へ合わせ直す
      });
      return btn;
    };
    const followBtn = mk(true);
    const fixedBtn = mk(false);
    const paint = (): void => {
      const on = isFollowing();
      followBtn.classList.toggle('active', on);
      fixedBtn.classList.toggle('active', !on);
      followBtn.setAttribute('aria-pressed', String(on));
      fixedBtn.setAttribute('aria-pressed', String(!on));
    };
    paint();
    toggle.append(followBtn, fixedBtn);
    bar.append(toggle);
  }

  const spacer = document.createElement('span');
  spacer.className = 'detached-tab-spacer';
  bar.append(spacer);

  const hub = document.createElement('button');
  hub.type = 'button';
  hub.className = 'detached-hub-open-btn';
  hub.textContent = '⊞ Hub';
  hub.title = t('detached_hub_tooltip');
  hub.addEventListener('click', focusOrOpenHub);
  bar.append(hub);
  return bar;
}

function focusOrOpenHub(): void {
  const opener = window.opener as Window | null;
  try {
    if (opener && !opener.closed) { opener.focus(); return; }
  } catch (_) { /* 開き元へ触れなければ新しく開く */ }
  window.open('/', '_blank');
}

async function askMain(msg: Parameters<typeof requestMainWindow>[0]): Promise<void> {
  const ok = await requestMainWindow(msg);
  if (!ok) showToast(t('detached_main_window_missing'), undefined, 4000);
}

// ─── ファイルの別窓（tab=file）─────────────────────────────────────────────

function showFileInWindow(): void {
  if (!fileHost || !fileTarget) return;
  // 差し替えのたびに載せ直す（ファイルごとに session と起点が違いうるため、bind からやり直す）
  const cwd = fileTarget.cwd || dirnameForPath(fileTarget.path);
  mountSingleFilePreview(fileHost, fileTarget.path, fileTarget.sessionId || '', cwd);
  updateBar();
}

/** 開き元（openDetachedFileWindow）から、表示するファイルの差し替えを受ける。 */
function onOpenFileMessage(event: MessageEvent): void {
  if (event.origin !== window.location.origin) return;
  const msg = event.data as OpenFileMessage | null;
  if (!msg || typeof msg !== 'object' || msg.type !== OPEN_FILE_MESSAGE_TYPE) return;
  if (typeof msg.path !== 'string' || !msg.path) return;
  // 編集中の内容を黙って捨てない
  if ((fileHost as any)?._filesPreview?.isEditing?.()) {
    showToast(t('files_window_editing'), undefined, 5000);
    return;
  }
  const sessionId = Number(msg.sessionId);
  fileTarget = {
    path: msg.path,
    sessionId: Number.isInteger(sessionId) && sessionId > 0 ? sessionId : 0,
    cwd: typeof msg.cwd === 'string' ? msg.cwd : '',
  };
  // 窓を読み直したときも同じファイルが出るよう、URL を今のファイルへ合わせる
  try {
    history.replaceState(history.state, '', buildDetachedFileUrl(fileTarget.path, fileTarget.sessionId, fileTarget.cwd));
  } catch (_) { /* 失敗しても表示は差し替わっている */ }
  showFileInWindow();
}

function initDetachedWindow(params: DetachedTabParams): void {
  const { tab, sessionId } = params;
  detachedTab = tab;
  initialSessionId = sessionId;
  if (tab === 'file') fileTarget = { path: params.path, sessionId, cwd: params.cwd };
  document.body.classList.add('detached-tab-mode');
  for (const id of HIDE_IDS) {
    const el = document.getElementById(id);
    if (el) { el.hidden = true; el.style.display = 'none'; }
  }

  const column = document.getElementById('terminal-column');
  // 見出しと作業メモは文言を使うので、辞書が読み込まれてから組み立てる
  // （i18n-ready はこのモジュールの評価より必ず後に来る）。
  whenI18nReady(() => {
    const bar = buildBar(tab);
    if (column) column.prepend(bar);
    else document.body.prepend(bar);
    if (tab === 'memo') {
      const host = document.createElement('div');
      host.id = 'detached-memo-host';
      if (column) column.append(host); else document.body.append(host);
      mountMemoWindow(host);
    } else if (tab === 'file') {
      fileHost = document.createElement('div');
      fileHost.id = 'detached-file-host';
      if (column) column.append(fileHost); else document.body.append(fileHost);
      showFileInWindow();
    }
    updateBar();
  });

  if (tab === 'memo' || tab === 'file') {
    // どちらも display-area（タブの表示面）を使わず、自前の入れ物に出す
    const stack = document.getElementById('display-stack');
    if (stack) { stack.hidden = true; stack.style.display = 'none'; }
    if (tab === 'file') window.addEventListener('message', onOpenFileMessage);
  } else if (!SESSION_BOUND_DETACHED_TABS.has(tab)) {
    // セッションに依存しないタブはセッションが 1 つも無くても出す。
    setActiveTab(null, tab);
    if (tab === 'multi') window.multiPaneManager?.picker?.hide?.();
  }

  setDetachedActivateHandler(onAppActivate);

  // 本体でしかできない操作は本体の窓へ頼む。
  // サブエージェント木の子カード（app.ts のリスナーは activateSession を呼ぶが、別窓では効かない）
  window.addEventListener('orchestration-dashboard-open-session', (event: Event) => {
    const sid = Number((event as CustomEvent<{ sessionID?: number }>).detail?.sessionID);
    if (Number.isInteger(sid) && sid > 0) void askMain({ type: 'open-session', sessionId: sid });
  });

  onWindowMessage((msg) => {
    if (msg.type === 'active-session') onFollowMessage(msg.sessionId);
  });
  if (SESSION_BOUND_DETACHED_TABS.has(tab) && isFollowing()) postWindowMessage({ type: 'hello' });
}

function whenI18nReady(fn: () => void): void {
  document.addEventListener('i18n-ready', fn, { once: true });
}

// ─── 本体の窓 ───────────────────────────────────────────────────────────────

let lastPostedSessionId: number | null | undefined;

function postActiveSession(force = false): void {
  if (!force && lastPostedSessionId === activeSessionId) return;
  lastPostedSessionId = activeSessionId;
  postWindowMessage({ type: 'active-session', sessionId: activeSessionId });
}

function initMainWindow(): void {
  // set_activeSessionId() が毎回出すイベント。選択が変わったときだけ流す。
  document.addEventListener('session-usage-target-changed', () => postActiveSession());
  onWindowMessage((msg) => {
    if (msg.type === 'hello') {
      postActiveSession(true);
    } else if (msg.type === 'open-session') {
      if (!sessions.has(msg.sessionId)) return;
      ackWindowRequest(msg.requestId);
      activateSession(msg.sessionId);
      try { window.focus(); } catch (_) { /* 前に出せなくても切り替えは済んでいる */ }
    } else if (msg.type === 'open-spawn') {
      ackWindowRequest(msg.requestId);
      if (msg.prompt !== undefined) openSpawnPanelWith({ cwd: msg.cwd, prompt: msg.prompt });
      else (window as any).openSpawnFor?.(msg.provider || 'claude', msg.cwd);
      try { window.focus(); } catch (_) { /* 同上 */ }
    }
  });
  wireOpenInWindowMenus();
}

// 右クリックの「別窓で開く」（C5）。ターミナルは既存の別窓グリッドがあるので付けない。
function showOpenInWindowMenu(event: MouseEvent, tab: DetachedTabName): void {
  event.preventDefault();
  event.stopPropagation();
  const popup = getOrCreatePathPopup();
  popup.replaceChildren();
  popup.hidden = false;
  renderPathPopupItems(popup, [{
    icon: '🗗',
    key: 'detached_open_in_window',
    action: () => {
      if (!openDetachedTabWindow(tab, activeSessionId)) showToast(t('detached_popup_blocked'), undefined, 6000);
    },
  }], event.clientX, event.clientY);
}

function wireOpenInWindowMenus(): void {
  document.querySelectorAll<HTMLElement>('#unified-tab-bar .view-tab').forEach((btn) => {
    const tab = btn.dataset.tab as DetachedTabName;
    if (!DETACHED_TAB_NAMES.includes(tab) || tab === 'memo') return;
    btn.addEventListener('contextmenu', (e) => showOpenInWindowMenu(e, tab));
  });
  document.querySelectorAll<HTMLElement>('[data-open-memo]').forEach((btn) => {
    btn.addEventListener('contextmenu', (e) => showOpenInWindowMenu(e, 'memo'));
  });
}

// ─── 入口 ─────────────────────────────────────────────────────────────────

export function initDetachedTabMode(): void {
  const params = detachedTabParams();
  if (params) initDetachedWindow(params);
  else if (isMainHubWindow()) initMainWindow();
}

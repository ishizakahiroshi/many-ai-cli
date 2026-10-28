// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { apiFetch, showToast, token } from './util.js';
import { activeSessionId, sessions, terminals } from './state.js';
import { basenameForPath, copyPathText } from './path-links.js';
import { appendComposerAttachment, captureComposerOwner, inputEl, isComposerOwnerLive, isInteractiveFocusTarget, removeComposerAttachment, stagePastedText, updateInputAffordance } from '../app.js';
import { pushMessage } from './chat-history.js';
import { openLightbox } from './lightbox.js';
import { isFetchableRemotePath, isRemoteViewablePath, openRemoteFileView } from './remote-file-view.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- ファイル転送 (attach) ----

export const attachDropZone = document.getElementById('attach-drop-zone');
export const attachPanel = document.getElementById('attach-panel');
export const attachFileInput = document.getElementById('attach-file-input');
export const attachThumbnails = document.getElementById('attach-thumbnails');
export const attachClearBtn = document.getElementById('attach-clear-btn');
// B2: スマホ専用カメラ撮影 input/btn（PC は CSS で非表示）。
export const attachCameraBtn = document.getElementById('attach-camera-btn');
export const attachCameraInput = document.getElementById('attach-camera-input');
export const pendingAttachFiles = []; // {buf, filename, entry, wrapper} — ステージング済み未送信ファイル
export const MAX_ATTACH_BYTES = 8 * 1024 * 1024;

export function isImageFile(file) {
  return file.type.startsWith('image/');
}

export function updateAttachClearBtn() {
  if (!attachThumbnails) return;
  const hasAttachments = attachThumbnails.querySelectorAll('.attach-thumb-wrapper').length > 0;
  attachPanel?.classList.toggle('has-attachments', hasAttachments);
  if (attachClearBtn) attachClearBtn.hidden = !hasAttachments;
}

if (attachClearBtn) {
  attachClearBtn.addEventListener('click', () => {
    if (!attachThumbnails) return;
    pendingAttachFiles.length = 0;
    attachThumbnails.querySelectorAll('.attach-thumb-wrapper').forEach(wrapper => {
      const img = wrapper.querySelector('img');
      if (img) URL.revokeObjectURL(img.src);
      wrapper.remove();
    });
    updateAttachClearBtn();
    updateInputAffordance();
  });
}

// 📁: アクティブセッションの作業ディレクトリパスをクリップボードへコピーするだけの常設ボタン。
// エクスプローラーを自前で開かない（利用者が常駐させている既存ウィンドウへ貼る運用のため）。
export const attachCopyCwdBtn = document.getElementById('attach-copy-cwd-btn');
if (attachCopyCwdBtn) {
  attachCopyCwdBtn.addEventListener('click', () => {
    const cwd = activeSessionId !== null ? sessions.get(activeSessionId)?.cwd : '';
    if (!cwd) return;
    void copyPathText(cwd, attachCopyCwdBtn).catch(() => {});
  });
}

type AgentLogResponse = {
  available?: boolean;
  path?: string;
  label?: string;
  reason?: string;
};

export const agentLogBtn = document.getElementById('agent-log-btn') as HTMLButtonElement | null;
let agentLogPopup: HTMLDivElement | null = null;

function closeAgentLogPopup() {
  if (agentLogPopup) agentLogPopup.hidden = true;
  agentLogBtn?.setAttribute('aria-expanded', 'false');
}

function getAgentLogPopup() {
  if (agentLogPopup) return agentLogPopup;
  const popup = document.createElement('div');
  popup.className = 'agent-log-popup';
  popup.hidden = true;
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('agent_log_button'));
  popup.addEventListener('click', (e) => e.stopPropagation());
  document.body.appendChild(popup);
  agentLogPopup = popup;
  return popup;
}

function showAgentLogPopup(anchor: HTMLButtonElement, sessionId: number, detail: AgentLogResponse) {
  const path = String(detail.path || '');
  if (!path) return;
  const popup = getAgentLogPopup();
  popup.replaceChildren();

  const title = document.createElement('div');
  title.className = 'agent-log-popup-title';
  title.textContent = detail.label || t('agent_log_location');
  const pathEl = document.createElement('code');
  pathEl.className = 'agent-log-popup-path';
  pathEl.textContent = path;
  const actions = document.createElement('div');
  actions.className = 'agent-log-popup-actions';

  const copyBtn = document.createElement('button');
  copyBtn.type = 'button';
  copyBtn.className = 'ui-button ui-button--quiet ui-button--compact';
  copyBtn.textContent = `📋 ${t('agent_log_copy_path')}`;
  copyBtn.addEventListener('click', () => {
    void copyPathText(path, copyBtn).catch(() => showToast(t('agent_log_copy_error'), copyBtn));
  });
  const openBtn = document.createElement('button');
  openBtn.type = 'button';
  openBtn.className = 'ui-button ui-button--secondary ui-button--compact';
  openBtn.textContent = `📁 ${t('agent_log_open_folder')}`;
  openBtn.addEventListener('click', async () => {
    try {
      const res = await apiFetch(`/api/agent-log/open?session_id=${encodeURIComponent(sessionId)}`, { method: 'POST' });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
    } catch (_) {
      showToast(t('agent_log_open_error'), openBtn);
    }
  });
  actions.append(copyBtn, openBtn);
  popup.append(title, pathEl, actions);
  popup.hidden = false;
  const rect = anchor.getBoundingClientRect();
  const width = popup.offsetWidth;
  popup.style.left = `${Math.max(8, Math.min(window.innerWidth - width - 8, rect.right - width))}px`;
  popup.style.top = `${Math.max(8, rect.top - popup.offsetHeight - 8)}px`;
  anchor.setAttribute('aria-expanded', 'true');
}

if (agentLogBtn) {
  agentLogBtn.addEventListener('click', async (event) => {
    event.stopPropagation();
    const sessionId = activeSessionId;
    if (sessionId === null) {
      showToast(t('agent_log_no_session'), agentLogBtn);
      return;
    }
    try {
      const res = await apiFetch(`/api/agent-log?session_id=${encodeURIComponent(sessionId)}`);
      const detail = await res.json() as AgentLogResponse;
      if (!res.ok || !detail.available) {
        showToast(detail.reason || t('agent_log_unavailable'), agentLogBtn);
        return;
      }
      showAgentLogPopup(agentLogBtn, sessionId, detail);
    } catch (_) {
      showToast(t('agent_log_unavailable'), agentLogBtn);
    }
  });
  document.addEventListener('click', closeAgentLogPopup);
  window.addEventListener('resize', closeAgentLogPopup);
}

// ---- 添付の送り先（シェルのセッションで ssh した先） ----
// シェルのセッションで ssh して入った先へ、添付（画像・ファイル・長文ペースト）を届けるための「送り先」。
// Hub のある PC に保存したパスは ssh 先からは開けないので、送り先を決めたセッションだけ、
// Hub が ssh で同じ内容を送り、送り先のパスを入力欄へ入れる（rust/src/files/remote_attach.rs）。
//
// 送り先はこのブラウザに「セッション番号＋開始時刻」の組で覚える。番号だけで覚えると、
// Hub の再起動で番号が使い回されたとき、無関係なセッションの添付が ssh 先へ送られてしまう。
const REMOTE_TARGETS_KEY = 'remoteAttachTargetsV1';
const MAX_REMOTE_TARGETS = 50;

// 1 セッションにつき 1 件。target があれば「その ssh 先へ送る」、target が空で local が true なら
// 「このPCで読むと決めた」。記録が無い（または target も local も無い）のは「まだ選んでいない」。
// 以前は「このPCに戻す」で記録そのものを消していたので、この 2 つを区別できなかった。
// 既存の保存値（key と target だけ）はそのまま読める。
type RemoteTargetEntry = { key: string; target: string; local?: boolean };

function remoteTargetKey(sessionId: number): string | null {
  const session = sessions.get(sessionId);
  if (!session || session.provider !== 'shell') return null;
  return `${sessionId}|${String(session.started_at || '')}`;
}

// localStorage が使えない（書けない）ときだけ、このページを開いている間の分を持つ。
// 持たないと、選んでも「まだ選んでいない」のままになり、送るたびに窓が出続けて添付を送れなくなる。
let remoteTargetsInMemory: RemoteTargetEntry[] | null = null;

function loadRemoteTargets(): RemoteTargetEntry[] {
  if (remoteTargetsInMemory) return remoteTargetsInMemory;
  try {
    const parsed = JSON.parse(localStorage.getItem(REMOTE_TARGETS_KEY) || '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((e) => e && typeof e.key === 'string' && typeof e.target === 'string');
  } catch (_) {
    return [];
  }
}

function storeRemoteTargets(entries: RemoteTargetEntry[]): void {
  const kept = entries.slice(-MAX_REMOTE_TARGETS);
  try {
    localStorage.setItem(REMOTE_TARGETS_KEY, JSON.stringify(kept));
    remoteTargetsInMemory = null;
  } catch (_) {
    remoteTargetsInMemory = kept;
  }
}

function remoteAttachEntry(sessionId: number | null): RemoteTargetEntry | undefined {
  if (sessionId === null) return undefined;
  const key = remoteTargetKey(sessionId);
  if (!key) return undefined;
  return loadRemoteTargets().find((e) => e.key === key);
}

/** 送り先が無い（このPCで読む／まだ選んでいない）ときは空文字。シェル以外のセッションは常に空文字。 */
export function remoteAttachTarget(sessionId: number | null): string {
  return remoteAttachEntry(sessionId)?.target || '';
}

/**
 * シェルのセッションで、添付の送り先をまだ選んでいない（ssh 先も「このPCで読む」も決めていない）か。
 * シェル以外のセッションは常に false。選ばないまま添付を送ると、ssh 先から読めない PC 上のパスが入る。
 */
export function isRemoteAttachChoicePending(sessionId: number | null): boolean {
  if (sessionId === null || !remoteTargetKey(sessionId)) return false;
  const entry = remoteAttachEntry(sessionId);
  return !(entry && (entry.target || entry.local === true));
}

/** target が空なら「このPCで読む」を選んだこととして記録する（記録は消さない）。 */
function setRemoteAttachTarget(sessionId: number, target: string): void {
  const key = remoteTargetKey(sessionId);
  if (!key) return;
  const entries = loadRemoteTargets().filter((e) => e.key !== key);
  entries.push(target ? { key, target } : { key, target: '', local: true });
  storeRemoteTargets(entries);
}

function recentRemoteTargets(): string[] {
  const seen = new Set<string>();
  for (const entry of loadRemoteTargets().slice().reverse()) {
    if (entry.target) seen.add(entry.target);
  }
  return [...seen];
}

// Hub 側（remote_attach.rs の validate_target）と同じ規則。ここでは送る前に打ち間違いを見せるためだけに使う。
export function isValidRemoteTarget(target: string): boolean {
  const host = target.startsWith('ssh://') ? target.slice('ssh://'.length) : target;
  return host.length > 0 && target.length <= 255 && !host.startsWith('-') && /^[A-Za-z0-9._@:[\]-]+$/.test(host);
}

export const attachRemoteBtn = document.getElementById('attach-remote-btn') as HTMLButtonElement | null;
let remoteAttachPopup: HTMLDivElement | null = null;
export const attachFetchBtn = document.getElementById('attach-fetch-btn') as HTMLButtonElement | null;
let remoteFetchPopup: HTMLDivElement | null = null;

function closeRemoteAttachPopup(): void {
  if (remoteAttachPopup) remoteAttachPopup.hidden = true;
  attachRemoteBtn?.setAttribute('aria-expanded', 'false');
}

function closeRemoteFetchPopup(): void {
  if (remoteFetchPopup) remoteFetchPopup.hidden = true;
  attachFetchBtn?.setAttribute('aria-expanded', 'false');
}

function syncRemoteAttachButton(): void {
  if (!attachRemoteBtn) return;
  const session = activeSessionId !== null ? sessions.get(activeSessionId) : undefined;
  const isShell = session?.provider === 'shell';
  attachRemoteBtn.hidden = !isShell;
  const target = remoteAttachTarget(activeSessionId);
  if (attachFetchBtn) attachFetchBtn.hidden = !target;
  if (!target) closeRemoteFetchPopup();
  if (!isShell) {
    closeRemoteAttachPopup();
    return;
  }
  // 長い接続先で添付欄を押し広げない。全体は押して開く窓の入力欄で見える。
  const shown = target.length > 24 ? `${target.slice(0, 23)}…` : target;
  attachRemoteBtn.textContent = target
    ? t('remote_attach_button_remote', { target: shown })
    : isRemoteAttachChoicePending(activeSessionId)
      ? t('remote_attach_button_unset')
      : t('remote_attach_button_local');
}

type RemoteAttachPopupOpts = {
  /** 添付を送ろうとして止めたとき。送る前に選んでもらう説明を出す。 */
  beforeSend?: boolean;
  /** 送り先（ssh 先か「このPCで読む」）を決めて窓を閉じた後に呼ぶ。決めずに閉じたら呼ばない。 */
  onDecided?: () => void;
};

function openRemoteAttachPopup(anchor: HTMLButtonElement, sessionId: number, opts: RemoteAttachPopupOpts = {}): void {
  if (!remoteAttachPopup) {
    remoteAttachPopup = document.createElement('div');
    remoteAttachPopup.className = 'agent-log-popup remote-attach-popup';
    remoteAttachPopup.hidden = true;
    remoteAttachPopup.setAttribute('role', 'dialog');
    remoteAttachPopup.setAttribute('aria-label', t('remote_attach_title'));
    remoteAttachPopup.addEventListener('click', (e) => e.stopPropagation());
    document.body.appendChild(remoteAttachPopup);
  }
  const box = remoteAttachPopup;
  box.replaceChildren();

  const title = document.createElement('div');
  title.className = 'agent-log-popup-title';
  title.textContent = t('remote_attach_title');
  const hint = document.createElement('div');
  hint.className = 'agent-log-popup-title remote-attach-hint';
  hint.textContent = t('remote_attach_hint');
  // 添付を送ろうとして止めたときは、なぜ今選ぶのか（選ばないと PC 上のパスが入る）を先に見せる。
  let choose: HTMLDivElement | null = null;
  if (opts.beforeSend) {
    choose = document.createElement('div');
    choose.className = 'agent-log-popup-title remote-attach-hint';
    choose.setAttribute('role', 'status');
    choose.textContent = t('remote_attach_choose_first');
  }

  const listId = 'remote-attach-recent';
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'ui-input remote-attach-input';
  input.value = remoteAttachTarget(sessionId);
  input.placeholder = t('remote_attach_placeholder');
  input.spellcheck = false;
  input.autocomplete = 'off';
  input.setAttribute('list', listId);
  input.setAttribute('aria-label', t('remote_attach_title'));
  const recent = document.createElement('datalist');
  recent.id = listId;
  for (const target of recentRemoteTargets()) {
    const option = document.createElement('option');
    option.value = target;
    recent.appendChild(option);
  }

  const save = () => {
    const target = input.value.trim();
    if (target && !isValidRemoteTarget(target)) {
      input.setAttribute('aria-invalid', 'true');
      showToast(t('remote_attach_invalid'), input);
      return;
    }
    setRemoteAttachTarget(sessionId, target);
    closeRemoteAttachPopup();
    syncRemoteAttachButton();
    showToast(target ? t('remote_attach_saved', { target }) : t('remote_attach_cleared'), anchor);
    // 一度だけ呼ぶ（窓を閉じた後なので、同じ窓から 2 回目の決定は来ない）。
    opts.onDecided?.();
  };
  input.addEventListener('input', () => input.removeAttribute('aria-invalid'));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault();
      save();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeRemoteAttachPopup();
      // 送る途中で開いたときは、入力欄へ戻す（何も送っておらず、入力欄の中身はそのまま）。
      if (opts.beforeSend) inputEl.focus();
      else anchor.focus();
    }
  });

  const actions = document.createElement('div');
  actions.className = 'agent-log-popup-actions';
  const saveBtn = document.createElement('button');
  saveBtn.type = 'button';
  saveBtn.className = 'ui-button ui-button--primary ui-button--compact';
  saveBtn.textContent = t('remote_attach_save');
  saveBtn.addEventListener('click', save);
  const localBtn = document.createElement('button');
  localBtn.type = 'button';
  localBtn.className = 'ui-button ui-button--quiet ui-button--compact';
  // 送り先がまだ無いときの「戻す」は、戻る先が無いので「このPCで読む」と言う。
  localBtn.textContent = t(remoteAttachTarget(sessionId) ? 'remote_attach_use_local' : 'remote_attach_read_local');
  localBtn.addEventListener('click', () => {
    input.value = '';
    save();
  });
  actions.append(saveBtn, localBtn);

  box.append(title, ...(choose ? [choose] : []), hint, input, recent, actions);
  box.hidden = false;
  // スマホ幅では添付欄ごと隠れていて、ボタンの位置が取れない（全部 0）。そのときは入力欄の上に出す。
  let rect = anchor.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) rect = inputEl.getBoundingClientRect();
  const width = box.offsetWidth;
  box.style.left = `${Math.max(8, Math.min(window.innerWidth - width - 8, rect.left))}px`;
  box.style.top = `${Math.max(8, rect.top - box.offsetHeight - 8)}px`;
  anchor.setAttribute('aria-expanded', 'true');
  input.focus();
  input.select();
}

/**
 * シェルのセッションで送り先をまだ選んでいないまま添付を送ろうとしたとき、送る前に送り先の窓を開く
 * （呼び出し側は、true が返ったら何も送らず入力欄・チップ・添付もそのまま残す）。
 * 窓は今のクリックが終わってから開く（送信ボタンのクリックは document まで届き、開いた窓を閉じてしまうため）。
 * 利用者が ssh 先か「このPCで読む」を決めたら onDecided を呼ぶ。決めずに閉じたら呼ばない。
 * 窓を出せない（シェルのセッションでない・もう選んである・ボタンが無い）ときは false で、何もしない。
 */
export function askRemoteAttachChoice(sessionId: number, onDecided?: () => void): boolean {
  if (!attachRemoteBtn || !isRemoteAttachChoicePending(sessionId)) return false;
  setTimeout(() => {
    if (activeSessionId !== sessionId || !isRemoteAttachChoicePending(sessionId)) return;
    syncRemoteAttachButton();
    closeRemoteFetchPopup();
    openRemoteAttachPopup(attachRemoteBtn, sessionId, { beforeSend: true, onDecided });
  }, 0);
  return true;
}

if (attachRemoteBtn) {
  attachRemoteBtn.addEventListener('click', (event) => {
    event.stopPropagation();
    if (remoteAttachPopup && !remoteAttachPopup.hidden) {
      closeRemoteAttachPopup();
      return;
    }
    const sessionId = activeSessionId;
    if (sessionId === null || sessions.get(sessionId)?.provider !== 'shell') return;
    closeRemoteFetchPopup();
    openRemoteAttachPopup(attachRemoteBtn, sessionId);
  });
  document.addEventListener('click', closeRemoteAttachPopup);
  window.addEventListener('resize', closeRemoteAttachPopup);
  document.addEventListener('session-usage-target-changed', syncRemoteAttachButton);
  document.addEventListener('i18n-ready', syncRemoteAttachButton);
}

// ---- 送り先から取ってくる ----
// 送り先の AI が「ここに置いた」と答えたファイルを、送るときと同じ ssh で読み、ブラウザのダウンロードとして保存する
// （/api/files-download に remote_target を付けたときだけ Hub が ssh で取る。rust/src/files/remote_attach.rs）。
// Hub のある PC には何も書かない。保存先はこのブラウザのダウンロードの設定に従う。
const MAX_RECENT_FETCH_PATHS = 10;
const recentFetchPaths: string[] = [];

// 送り先のパスとして受けてよい形かの規則（isFetchableRemotePath）は、「開く」と共有するため remote-file-view.ts に置いた。

function rememberFetchPath(path: string): void {
  const index = recentFetchPaths.indexOf(path);
  if (index >= 0) recentFetchPaths.splice(index, 1);
  recentFetchPaths.unshift(path);
  recentFetchPaths.length = Math.min(recentFetchPaths.length, MAX_RECENT_FETCH_PATHS);
}

/** 成功したら true。失敗の理由（鍵・パス違い・大きすぎ）は利用者が直すものなので、Hub の detail をそのまま見せる。 */
export async function fetchRemoteFile(sessionId: number, remotePath: string, anchor: HTMLElement | null = null): Promise<boolean> {
  const target = remoteAttachTarget(sessionId);
  const path = remotePath.trim();
  if (!target) {
    showToast(t('remote_fetch_no_target'), anchor);
    return false;
  }
  if (!isFetchableRemotePath(path)) {
    showToast(t('remote_fetch_invalid'), anchor);
    return false;
  }
  rememberFetchPath(path);
  showToast(t('remote_fetch_started', { target }), anchor);
  try {
    const res = await apiFetch(
      `/api/files-download?session=${encodeURIComponent(sessionId)}&remote_target=${encodeURIComponent(target)}&path=${encodeURIComponent(path)}`
    );
    if (!res.ok) {
      const detail = await res.json().then((body) => String(body?.detail || ''), () => '');
      showToast(t('remote_fetch_failed', { target, detail: detail || `HTTP ${res.status}` }), undefined, 8000);
      return false;
    }
    const blob = await res.blob();
    const name = basenameForPath(path) || 'download';
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = name;
    a.hidden = true;
    document.body.appendChild(a);
    a.click();
    a.remove();
    // クリック直後に解放すると、保存が始まる前に URL が消えるブラウザがある。
    setTimeout(() => URL.revokeObjectURL(a.href), 60_000);
    showToast(t('remote_fetch_done', { name }), anchor);
    return true;
  } catch (_) {
    showToast(t('remote_fetch_failed', { target, detail: 'network error' }), undefined, 8000);
    return false;
  }
}

/**
 * ssh 先のファイルを、ダウンロードせずに読み取り専用の画面（モーダル）で開く。「取ってくる」と同じ Hub の経路で読む
 * （remote-file-view.ts）。画面の「取ってくる」ボタンから同じファイルを保存もできる。
 */
function openRemoteFile(sessionId: number, path: string): Promise<boolean> {
  return openRemoteFileView(sessionId, remoteAttachTarget(sessionId), path, { onFetch: () => { void fetchRemoteFile(sessionId, path); } });
}

/**
 * 端末に出たパスのメニューへ足す項目。送り先を決めたシェルのセッションで、送り先のパスとして読めるものだけ。
 * 「開く」（画面に表示できる種類のときだけ）を先に、「取ってくる」を後に並べる。
 */
export function remoteFetchMenuItems(sessionId: number | null, path: string): any[] {
  if (sessionId === null) return [];
  const target = remoteAttachTarget(sessionId);
  if (!target || !isFetchableRemotePath(path)) return [];
  // メニューは押した時点で閉じる（取り終わるまで開いたままにしない）。経過と結果はトーストで出す。
  const items: any[] = [];
  if (isRemoteViewablePath(path)) {
    items.push({ icon: '👁️', label: t('remote_view_menu', { target }), action: () => { void openRemoteFile(sessionId, path); } });
  }
  items.push({ icon: '⬇️', label: t('remote_fetch_menu', { target }), action: () => { void fetchRemoteFile(sessionId, path); } });
  return items;
}

function openRemoteFetchPopup(anchor: HTMLButtonElement, sessionId: number): void {
  if (!remoteFetchPopup) {
    remoteFetchPopup = document.createElement('div');
    remoteFetchPopup.className = 'agent-log-popup remote-attach-popup';
    remoteFetchPopup.hidden = true;
    remoteFetchPopup.setAttribute('role', 'dialog');
    remoteFetchPopup.addEventListener('click', (e) => e.stopPropagation());
    document.body.appendChild(remoteFetchPopup);
  }
  const box = remoteFetchPopup;
  box.setAttribute('aria-label', t('remote_fetch_title'));
  box.replaceChildren();

  const title = document.createElement('div');
  title.className = 'agent-log-popup-title';
  title.textContent = t('remote_fetch_title');
  const hint = document.createElement('div');
  hint.className = 'agent-log-popup-title remote-attach-hint';
  hint.textContent = t('remote_fetch_hint', { target: remoteAttachTarget(sessionId) });

  const listId = 'remote-fetch-recent';
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'ui-input remote-attach-input';
  input.value = recentFetchPaths[0] || '';
  input.placeholder = t('remote_fetch_placeholder');
  input.spellcheck = false;
  input.autocomplete = 'off';
  input.setAttribute('list', listId);
  input.setAttribute('aria-label', t('remote_fetch_title'));
  const recent = document.createElement('datalist');
  recent.id = listId;
  for (const path of recentFetchPaths) {
    const option = document.createElement('option');
    option.value = path;
    recent.appendChild(option);
  }

  const actions = document.createElement('div');
  actions.className = 'agent-log-popup-actions';
  const fetchBtn = document.createElement('button');
  fetchBtn.type = 'button';
  fetchBtn.className = 'ui-button ui-button--primary ui-button--compact';
  fetchBtn.textContent = t('remote_fetch_submit');
  // 「開く」は画面に表示するだけ（保存しない）。「取ってくる」の隣に、先に並べる。
  const openBtn = document.createElement('button');
  openBtn.type = 'button';
  openBtn.className = 'ui-button ui-button--secondary ui-button--compact';
  openBtn.textContent = t('remote_view_submit');
  actions.append(openBtn, fetchBtn);

  const submit = async () => {
    if (fetchBtn.disabled || openBtn.disabled) return;
    const path = input.value.trim();
    if (!isFetchableRemotePath(path)) {
      input.setAttribute('aria-invalid', 'true');
      showToast(t('remote_fetch_invalid'), input);
      return;
    }
    fetchBtn.disabled = true;
    openBtn.disabled = true;
    fetchBtn.textContent = t('remote_fetch_busy');
    const ok = await fetchRemoteFile(sessionId, path, anchor);
    fetchBtn.disabled = false;
    openBtn.disabled = false;
    fetchBtn.textContent = t('remote_fetch_submit');
    if (ok) closeRemoteFetchPopup();
  };
  const openInView = async () => {
    if (fetchBtn.disabled || openBtn.disabled) return;
    const path = input.value.trim();
    if (!isFetchableRemotePath(path)) {
      input.setAttribute('aria-invalid', 'true');
      showToast(t('remote_fetch_invalid'), input);
      return;
    }
    // 開けない種類・失敗の知らせは openRemoteFileView が出す。画面が出ている間は二重に押せないようにする。
    rememberFetchPath(path);
    fetchBtn.disabled = true;
    openBtn.disabled = true;
    const ok = await openRemoteFile(sessionId, path);
    fetchBtn.disabled = false;
    openBtn.disabled = false;
    if (ok) closeRemoteFetchPopup();
  };
  fetchBtn.addEventListener('click', () => { void submit(); });
  openBtn.addEventListener('click', () => { void openInView(); });
  input.addEventListener('input', () => input.removeAttribute('aria-invalid'));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.isComposing) {
      e.preventDefault();
      void submit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeRemoteFetchPopup();
      anchor.focus();
    }
  });

  box.append(title, hint, input, recent, actions);
  box.hidden = false;
  const rect = anchor.getBoundingClientRect();
  const width = box.offsetWidth;
  box.style.left = `${Math.max(8, Math.min(window.innerWidth - width - 8, rect.left))}px`;
  box.style.top = `${Math.max(8, rect.top - box.offsetHeight - 8)}px`;
  anchor.setAttribute('aria-expanded', 'true');
  input.focus();
  input.select();
}

if (attachFetchBtn) {
  attachFetchBtn.addEventListener('click', (event) => {
    event.stopPropagation();
    if (remoteFetchPopup && !remoteFetchPopup.hidden) {
      closeRemoteFetchPopup();
      return;
    }
    const sessionId = activeSessionId;
    if (sessionId === null || !remoteAttachTarget(sessionId)) return;
    closeRemoteAttachPopup();
    openRemoteFetchPopup(attachFetchBtn, sessionId);
  });
  document.addEventListener('click', closeRemoteFetchPopup);
  window.addEventListener('resize', closeRemoteFetchPopup);
}

// 下の入力欄以外の文字入力欄（作業メモ・ファイル編集・コミット本文など）への貼り付けは
// その欄に任せる。ここで拾うと、長文はチップ・画像は添付として下の入力欄へ持っていかれる
// （bugfix_memo-paste-goes-to-composer_2026-09-30.md）。端末（xterm）は自分で伝播を止めるので届かない。
const TEXT_INPUT_TYPES = new Set(['text', 'search', 'url', 'email', 'tel', 'password', 'number']);
function isOtherTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement) || target === inputEl) return false;
  if (target.isContentEditable || target instanceof HTMLTextAreaElement) return true;
  return target instanceof HTMLInputElement && TEXT_INPUT_TYPES.has(target.type);
}

window.addEventListener('paste', (e) => {
  if (activeSessionId === null) return;
  if (isOtherTextField(e.target)) return;
  const items = e.clipboardData?.items;
  if (!items) return;

  // ファイルを優先（画像 or その他ファイル）
  let hasFile = false;
  for (const item of items) {
    if (item.kind !== 'file') continue;
    const file = item.getAsFile();
    if (!file) continue;
    hasFile = true;
    if (isImageFile(file)) stageAttach(file, { normalize: true });
    else stageFileAttach(file);
  }
  if (hasFile) return;

  // 長いテキストはチップに折りたたむ
  const text = e.clipboardData?.getData('text');
  if (text && stagePastedText(text)) e.preventDefault();
});

if (attachDropZone) {
  attachDropZone.addEventListener('dragover', (e) => {
    e.preventDefault();
    attachDropZone.classList.add('dragover');
  });
  attachDropZone.addEventListener('dragleave', () => attachDropZone.classList.remove('dragover'));
  attachDropZone.addEventListener('drop', (e) => {
    e.preventDefault();
    attachDropZone.classList.remove('dragover');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
  attachDropZone.addEventListener('click', () => attachFileInput?.click());
}

if (attachFileInput) {
  attachFileInput.addEventListener('change', () => {
    for (const file of attachFileInput.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
    attachFileInput.value = '';
  });
}

// B2: スマホからの「📷 撮影」ボタン → カメラ起動 input.click()。
// クリップボード画像と同じ normalize:true ルートで PNG 圧縮してから添付する。
if (attachCameraBtn && attachCameraInput) {
  attachCameraBtn.addEventListener('click', () => attachCameraInput?.click());
  attachCameraInput.addEventListener('change', () => {
    for (const file of attachCameraInput.files ?? []) {
      if (isImageFile(file)) stageAttach(file, { normalize: true });
    }
    attachCameraInput.value = '';
  });
}

// セッション内どこでもD&D（terminal-wrapper全体）
export const terminalWrapper = document.getElementById('terminal-wrapper');
if (terminalWrapper) {
  terminalWrapper.addEventListener('click', (e) => {
    if (activeSessionId === null) return;
    if (isInteractiveFocusTarget(e.target)) return;
    // DOM テキスト選択中は入力欄へフォーカスすると選択が消える
    const domSel = window.getSelection();
    if (domSel && !domSel.isCollapsed && String(domSel).length > 0) return;
    const xt = terminals.get(activeSessionId);
    if (!xt?.term.hasSelection()) inputEl.focus();
  });

  // xterm.js canvas が click イベントを止める場合のフォールバック:
  // mouseup は canvas からもバブルするため、こちらで確実にフォーカスを戻す。
  // ただし Grok 会話履歴ビューア等の DOM オーバーレイ上では、xterm 未選択の
  // まま input にフォーカスするとブラウザ選択が消えてコピー不能になる。
  document.getElementById('terminal-area-wrapper')?.addEventListener('mouseup', (e) => {
    if (activeSessionId === null) return;
    if (isInteractiveFocusTarget(e.target)) return;
    const domSel = window.getSelection();
    if (domSel && !domSel.isCollapsed && String(domSel).length > 0) return;
    const xt = terminals.get(activeSessionId);
    // 50ms 待って xterm の選択状態が確定してから判定
    setTimeout(() => {
      if (isInteractiveFocusTarget(e.target)) return;
      const still = window.getSelection();
      if (still && !still.isCollapsed && String(still).length > 0) return;
      if (!xt?.term.hasSelection()) inputEl.focus();
    }, 50);
  });

  terminalWrapper.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    terminalWrapper.classList.add('drag-active');
  });
  terminalWrapper.addEventListener('dragleave', (e) => {
    if (!terminalWrapper.contains(e.relatedTarget)) {
      terminalWrapper.classList.remove('drag-active');
    }
  });
  terminalWrapper.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  terminalWrapper.addEventListener('drop', (e) => {
    e.preventDefault();
    terminalWrapper.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

// チャットペインへの D&D
export const chatPane = document.getElementById('chat-pane');
if (chatPane) {
  chatPane.addEventListener('dragenter', (e) => {
    if (!e.dataTransfer?.types.includes('Files')) return;
    e.preventDefault();
    chatPane.classList.add('drag-active');
  });
  chatPane.addEventListener('dragleave', (e) => {
    if (!chatPane.contains(e.relatedTarget)) {
      chatPane.classList.remove('drag-active');
    }
  });
  chatPane.addEventListener('dragover', (e) => {
    if (e.dataTransfer?.types.includes('Files')) e.preventDefault();
  });
  chatPane.addEventListener('drop', (e) => {
    e.preventDefault();
    chatPane.classList.remove('drag-active');
    if (activeSessionId === null) return;
    for (const file of e.dataTransfer?.files ?? []) {
      if (isImageFile(file)) stageAttach(file);
      else stageFileAttach(file);
    }
  });
}

export async function stageAttach(file, opts: any = {}) {
  const owner = captureComposerOwner();
  if (!owner) return;
  const normalized = opts.normalize ? await normalizeAttachImage(file) : file;
  const buf = await normalized.arrayBuffer();
  if (!isComposerOwnerLive(owner)) return;
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry: any = {};
  const wrapper = addAttachThumbnail(normalized, () => {
    removeComposerAttachment(owner, entry);
  }, owner.sessionId === activeSessionId);
  entry.wrapper = wrapper;
  appendComposerAttachment(owner, { buf, filename: normalized.name || '', entry, wrapper });
  updateAttachClearBtn();
}

export async function stageFileAttach(file) {
  const owner = captureComposerOwner();
  if (!owner) return;
  const buf = await file.arrayBuffer();
  if (!isComposerOwnerLive(owner)) return;
  if (buf.byteLength > MAX_ATTACH_BYTES) {
    showToast(`Attachment too large: ${(buf.byteLength / (1024 * 1024)).toFixed(1)}MB (max 8MB)`);
    return;
  }
  const entry: any = {};
  const wrapper = addFileChip(file, () => {
    removeComposerAttachment(owner, entry);
  }, owner.sessionId === activeSessionId);
  entry.wrapper = wrapper;
  appendComposerAttachment(owner, { buf, filename: file.name || '', entry, wrapper });
  updateAttachClearBtn();
}

// クリップボード画像は元ファイル名がないことが多いため、PNG として保存する。
// PNG が大きすぎる場合は、容量に応じて段階的に縮小する。
// D&D/ファイル選択では元ファイルをそのまま送る。
export async function normalizeAttachImage(file) {
  let bmp: ImageBitmap | null = null;
  try {
    const maxEdge = 1568;
    bmp = await createImageBitmap(file);
    const w = bmp.width;
    const h = bmp.height;
    let scale = Math.min(1, maxEdge / Math.max(w, h));
    let lastBlob: Blob | null = null;

    for (let i = 0; i < 12; i++) {
      const outW = Math.max(1, Math.round(w * scale));
      const outH = Math.max(1, Math.round(h * scale));
      const canvas = document.createElement('canvas');
      canvas.width = outW;
      canvas.height = outH;
      const ctx = canvas.getContext('2d');
      if (!ctx) return file;
      ctx.drawImage(bmp, 0, 0, outW, outH);

      const blob = await new Promise<Blob | null>((resolve) => {
        canvas.toBlob(resolve, 'image/png');
      });
      if (!blob) return file;
      lastBlob = blob;
      if (blob.size <= MAX_ATTACH_BYTES) {
        const base = (file.name || 'image').replace(/\.[^.]+$/, '');
        return new File([blob], `${base}.png`, { type: 'image/png' });
      }

      const sizeRatio = MAX_ATTACH_BYTES / blob.size;
      const shrink = Math.max(0.35, Math.min(0.85, Math.sqrt(sizeRatio) * 0.9));
      scale *= shrink;
      if (outW === 1 && outH === 1) break;
    }

    if (lastBlob) {
      const base = (file.name || 'image').replace(/\.[^.]+$/, '');
      return new File([lastBlob], `${base}.png`, { type: 'image/png' });
    }
    return file;
  } catch (_) {
    return file;
  } finally {
    if (bmp) bmp.close();
  }
}

export function arrayBufferToBase64(buf) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const s = String(reader.result || '');
      const comma = s.indexOf(',');
      resolve(comma >= 0 ? s.slice(comma + 1) : s);
    };
    reader.onerror = () => reject(reader.error || new Error('base64 encode failed'));
    reader.readAsDataURL(new Blob([buf]));
  });
}

export async function flushPendingAttach(sessionId, capturedFiles?: any[]) {
  const toSend = capturedFiles || pendingAttachFiles.splice(0);
  if (toSend.length === 0) return [];
  const injects = [];
  // chatHistory 用: 送信に成功した添付の情報を集める
  const historyAttachments = [];
  // シェルのセッションで送り先（ssh 先）が決まっていれば、Hub が ssh で送って送り先のパスを返す。
  const remoteTarget = remoteAttachTarget(sessionId);
  const remoteQuery = remoteTarget ? `&remote_target=${encodeURIComponent(remoteTarget)}` : '';
  for (const { buf, filename, wrapper } of toSend) {
    try {
      const formData = new FormData();
      formData.append('file', new Blob([buf]), filename || 'blob');
      const res = await apiFetch(
        `/api/attach?session_id=${encodeURIComponent(sessionId)}${remoteQuery}`,
        { method: 'POST', body: formData }
      );
      if (!res.ok) {
        if (remoteTarget) {
          // ssh の失敗理由（鍵が無い・ホスト名違い等）は利用者が直すものなので、Hub の detail をそのまま見せる。
          const detail = await res.json().then((body) => String(body?.detail || ''), () => '');
          showToast(t('remote_attach_failed', { target: remoteTarget, detail: detail || `HTTP ${res.status}` }), undefined, 8000);
        } else {
          showToast(`Attachment failed: HTTP ${res.status}`);
        }
      } else {
        try {
          const data = await res.json();
          if (data && data.inject) injects.push(data.inject);
          const attachKind = /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(filename || '') ? 'image' : 'file';
          const blob = new Blob([buf]);
          historyAttachments.push({
            filename: filename || '',
            byteLength: (buf && buf.byteLength) || 0,
            kind: attachKind,
            path: data && data.saved_path ? data.saved_path : null,
            url: attachKind === 'image' ? URL.createObjectURL(blob) : null,
          });
        } catch (_) {
          showToast('Attachment response parse failed');
        }
      }
    } catch (_) {
      showToast('Attachment send failed');
    }
    if (wrapper) {
      wrapper.remove();
      updateAttachClearBtn();
    }
  }
  // chatHistory: attach を user/attach として 1 メッセージにまとめて push
  if (historyAttachments.length > 0) {
    pushMessage(sessionId, {
      role: 'user',
      kind: 'attach',
      attachments: historyAttachments,
      rawText: '',
    });
  }
  return injects;
}

export function addAttachThumbnail(file, onRemove, mount = true) {
  if (!attachThumbnails) return;
  const url = URL.createObjectURL(file);

  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper';

  const img = document.createElement('img');
  img.src = url;
  img.className = 'attach-thumb';
  img.title = (file.name || 'image') + t('expand_image');
  img.addEventListener('click', () => openLightbox(img.src));

  const removeBtn = document.createElement('button');
  removeBtn.className = 'ui-button ui-button--quiet ui-button--compact attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    URL.revokeObjectURL(url);
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(img);
  wrapper.appendChild(removeBtn);
  if (mount) attachThumbnails.prepend(wrapper);

  if (mount) {
    const wrappers = attachThumbnails.querySelectorAll('.attach-thumb-wrapper');
    for (let i = 10; i < wrappers.length; i++) {
      URL.revokeObjectURL(wrappers[i].querySelector('img').src);
      wrappers[i].remove();
    }
  }
  updateAttachClearBtn();
  return wrapper;
}

export function addFileChip(file, onRemove, mount = true) {
  if (!attachThumbnails) return;
  const wrapper = document.createElement('div');
  wrapper.className = 'attach-thumb-wrapper attach-file-chip';

  const label = document.createElement('span');
  label.className = 'attach-file-name';
  label.textContent = t('file_chip_label', { name: file.name || 'file' });
  label.title = file.name || 'file';

  const removeBtn = document.createElement('button');
  removeBtn.className = 'ui-button ui-button--quiet ui-button--compact attach-thumb-remove';
  removeBtn.textContent = t('remove');
  removeBtn.title = t('delete_attach');
  removeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    wrapper.remove();
    updateAttachClearBtn();
    onRemove?.();
  });

  wrapper.appendChild(label);
  wrapper.appendChild(removeBtn);
  if (mount) attachThumbnails.prepend(wrapper);
  updateAttachClearBtn();
  return wrapper;
}

// 本体は葉モジュール lightbox.ts（作業メモからも使うため）。既存の import 元を保つ。
export { openLightbox };

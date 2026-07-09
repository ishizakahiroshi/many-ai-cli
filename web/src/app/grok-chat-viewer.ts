// Grok 会話履歴ビューア: Grok Build CLI 自身が保存する chat_history.jsonl を
// Hub の /api/grok-history 経由で読み、user/assistant メッセージとして整形表示する。
// Grok は alt screen 上の全画面上書き描画で xterm のスクロールバックが育たないため、
// 生 PTY ログ再生（history-viewer.ts）では読める表示にならない。many-ai-cli 側では
// 何も保存せず、Grok が元々持つ構造化ログを読み取り専用で参照する。
import { showToast, ti18n, token } from './util.js';
import { sessions } from './state.js';

// 1 リクエストで取得するメッセージ件数（サーバ側上限 200 以内）
const GCV_PAGE_MESSAGES = 100;

let gcvRoot: any = null;
let gcvState: { sid: number; total: number; offset: number } | null = null;
let gcvLoading = false;

function gcvLabel(key: string, fallback: string): string {
  const v = ti18n(key);
  return (v && v !== key) ? v : fallback;
}

function ensureViewer() {
  if (gcvRoot) return gcvRoot;
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return null;
  const root = document.createElement('div');
  root.id = 'grok-chat-viewer';
  root.hidden = true;
  // document レベルの wheel 転送（terminal.ts）から除外し、ネイティブスクロールを生かす
  root.setAttribute('data-wheel-native', '');

  const header = document.createElement('div');
  header.className = 'gcv-header';
  const title = document.createElement('span');
  title.className = 'gcv-title';
  const range = document.createElement('span');
  range.className = 'gcv-range';
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'gcv-btn gcv-close';
  closeBtn.addEventListener('click', closeGrokChatViewer);
  header.appendChild(title);
  header.appendChild(range);
  header.appendChild(closeBtn);

  const body = document.createElement('div');
  body.className = 'gcv-body';
  const olderBtn = document.createElement('button');
  olderBtn.type = 'button';
  olderBtn.className = 'gcv-btn gcv-older';
  olderBtn.hidden = true;
  olderBtn.addEventListener('click', gcvLoadOlder);
  const list = document.createElement('div');
  list.className = 'gcv-list';
  body.appendChild(olderBtn);
  body.appendChild(list);

  root.appendChild(header);
  root.appendChild(body);
  wrapper.appendChild(root);
  gcvRoot = root;
  return root;
}

function gcvApplyLabels() {
  if (!gcvRoot) return;
  const set = (sel: string, key: string, fallback: string) => {
    const el = gcvRoot.querySelector(sel);
    if (el) el.textContent = gcvLabel(key, fallback);
  };
  set('.gcv-title', 'gcv_title', 'Grok 会話履歴（読み取り専用）');
  set('.gcv-close', 'gcv_close', '✕ 閉じる');
  set('.gcv-older', 'gcv_older', '↑ さらに前を読む');
}

function gcvUpdateRangeLabel() {
  if (!gcvRoot || !gcvState) return;
  const el = gcvRoot.querySelector('.gcv-range');
  if (!el) return;
  const shown = gcvState.total - gcvState.offset;
  el.textContent = `${shown} / ${gcvState.total}`;
}

async function gcvFetchPage(sid: number, offset: number, limit: number) {
  const params = new URLSearchParams({
    token,
    session_id: String(sid),
    limit: String(limit),
  });
  if (offset >= 0) params.set('offset', String(offset));
  const res = await fetch(`/api/grok-history?${params.toString()}`);
  if (!res.ok) throw new Error(`grok-history ${res.status}`);
  return res.json();
}

function gcvRenderMessage(msg: { role: string; text: string }) {
  const item = document.createElement('div');
  item.className = 'gcv-msg ' + (msg.role === 'user' ? 'gcv-user' : 'gcv-assistant');
  const role = document.createElement('div');
  role.className = 'gcv-role';
  role.textContent = msg.role === 'user'
    ? gcvLabel('gcv_role_user', 'あなた')
    : gcvLabel('gcv_role_assistant', 'Grok');
  const text = document.createElement('div');
  text.className = 'gcv-text';
  text.textContent = msg.text;
  item.appendChild(role);
  item.appendChild(text);
  return item;
}

function gcvSyncOlderBtn() {
  if (!gcvRoot || !gcvState) return;
  const olderBtn = gcvRoot.querySelector('.gcv-older');
  if (olderBtn) olderBtn.hidden = gcvState.offset <= 0;
}

async function gcvLoadLatest(sid: number) {
  if (gcvLoading) return;
  gcvLoading = true;
  try {
    const resp = await gcvFetchPage(sid, -1, GCV_PAGE_MESSAGES);
    if (!gcvState || gcvState.sid !== sid) return;
    gcvState.total = resp.total || 0;
    gcvState.offset = resp.offset || 0;
    const list = gcvRoot ? gcvRoot.querySelector('.gcv-list') : null;
    if (!list) return;
    list.textContent = '';
    const msgs = resp.messages || [];
    if (msgs.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'gcv-empty';
      empty.textContent = gcvLabel('gcv_empty', '表示できる会話がまだありません');
      list.appendChild(empty);
    }
    for (const m of msgs) list.appendChild(gcvRenderMessage(m));
    gcvSyncOlderBtn();
    gcvUpdateRangeLabel();
    const body = gcvRoot.querySelector('.gcv-body');
    if (body) body.scrollTop = body.scrollHeight;
  } catch (err) {
    console.warn('[grok-chat-viewer] load failed', err);
    showToast(gcvLabel('gcv_load_failed', 'Grok の会話履歴を取得できませんでした'));
    closeGrokChatViewer();
  } finally {
    gcvLoading = false;
  }
}

async function gcvLoadOlder() {
  if (!gcvState || gcvLoading || gcvState.offset <= 0) return;
  gcvLoading = true;
  try {
    const limit = Math.min(GCV_PAGE_MESSAGES, gcvState.offset);
    const offset = gcvState.offset - limit;
    const resp = await gcvFetchPage(gcvState.sid, offset, limit);
    if (!gcvState) return;
    gcvState.total = resp.total || gcvState.total;
    gcvState.offset = resp.offset ?? offset;
    const list = gcvRoot ? gcvRoot.querySelector('.gcv-list') : null;
    const body = gcvRoot ? gcvRoot.querySelector('.gcv-body') : null;
    if (!list || !body) return;
    // 先頭に差し込んでもスクロール位置（見えている行）が動かないよう補正する
    const prevHeight = body.scrollHeight;
    const frag = document.createDocumentFragment();
    for (const m of (resp.messages || [])) frag.appendChild(gcvRenderMessage(m));
    list.insertBefore(frag, list.firstChild);
    body.scrollTop += body.scrollHeight - prevHeight;
    gcvSyncOlderBtn();
    gcvUpdateRangeLabel();
  } catch (err) {
    console.warn('[grok-chat-viewer] load older failed', err);
    showToast(gcvLabel('gcv_load_failed', 'Grok の会話履歴を取得できませんでした'));
  } finally {
    gcvLoading = false;
  }
}

export function openGrokChatViewer(sid: number) {
  if (!sessions.has(sid)) return;
  const root = ensureViewer();
  if (!root) return;
  gcvApplyLabels();
  root.hidden = false;
  gcvState = { sid, total: 0, offset: 0 };
  gcvLoadLatest(sid);
}

export function isGrokChatViewerOpen(): boolean {
  return !!(gcvRoot && !gcvRoot.hidden);
}

export function closeGrokChatViewer() {
  if (gcvRoot) gcvRoot.hidden = true;
  gcvState = null;
}

// セッション切替・タブ切替時に閉じる（別セッションの履歴を誤表示しない）
export function resetGrokChatViewerForSessionChange() {
  closeGrokChatViewer();
}

// Escape で閉じる
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && gcvRoot && !gcvRoot.hidden) closeGrokChatViewer();
});

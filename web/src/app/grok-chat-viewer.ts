// Grok 会話履歴ビューア: Grok Build CLI 自身が保存する chat_history.jsonl を
// Hub の /api/grok-history 経由で読み、user/assistant メッセージとして整形表示する。
// Grok は alt screen 上の全画面上書き描画で xterm のスクロールバックが育たないため、
// 生 PTY ログ再生（history-viewer.ts）では読める表示にならない。many-ai-cli 側では
// 何も保存せず、Grok が元々持つ構造化ログを読み取り専用で参照する。
//
// パス・URL は Chat タブ相当のクリック／右クリック操作を提供する（読み取り専用でも
// ファイルを開く・コピーする導線は必要。textContent 直書きだとプレーン文字列のまま）。
import { showToast, ti18n, token } from './util.js';
import { sessions } from './state.js';
import { appendLinkedText } from './path-links.js';

// 1 リクエストで取得するメッセージ件数（サーバ側上限 200 以内）
const GCV_PAGE_MESSAGES = 100;

// Chat タブ (_appendPlainWithLinks) と同系の URL 検出。末尾句読点は本文側に残す。
const GCV_URL_RE = /(https?:\/\/[^\s<>"'`)\]]+)/g;

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

// URL とファイルパスをリンク化し、クリック／右クリックで開く・コピーできるようにする。
// パスは path-links.appendLinkedText（showPathPopup 付き）を流用。URL は別タブで開く <a>。
function gcvFillTextWithLinks(container: HTMLElement, raw: string, sessionId: number) {
  const text = String(raw || '');
  if (!text) {
    container.textContent = '';
    return;
  }

  const parts: Array<{ kind: 'url' | 'text'; value: string }> = [];
  GCV_URL_RE.lastIndex = 0;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = GCV_URL_RE.exec(text)) !== null) {
    if (m.index > last) parts.push({ kind: 'text', value: text.slice(last, m.index) });
    let token = m[0];
    let trail = '';
    while (token.length > 0 && /[.,;:!?)\]}>]/.test(token[token.length - 1]!)) {
      trail = token[token.length - 1] + trail;
      token = token.slice(0, -1);
    }
    if (token) parts.push({ kind: 'url', value: token });
    if (trail) parts.push({ kind: 'text', value: trail });
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push({ kind: 'text', value: text.slice(last) });
  if (parts.length === 0) {
    appendLinkedText(container, text, sessionId);
    return;
  }

  container.textContent = '';
  for (const part of parts) {
    if (part.kind === 'url') {
      const a = document.createElement('a');
      a.className = 'gcv-url-link';
      a.href = part.value;
      a.target = '_blank';
      a.rel = 'noopener noreferrer';
      a.textContent = part.value;
      container.appendChild(a);
      continue;
    }
    // appendLinkedText は container を空にしてから埋めるため、区間ごとに一時ノードへ書く
    const span = document.createElement('span');
    appendLinkedText(span, part.value, sessionId);
    while (span.firstChild) container.appendChild(span.firstChild);
  }
}

function gcvRenderMessage(msg: { role: string; text: string }, sessionId: number) {
  const item = document.createElement('div');
  item.className = 'gcv-msg ' + (msg.role === 'user' ? 'gcv-user' : 'gcv-assistant');
  const head = document.createElement('div');
  head.className = 'gcv-msg-head';
  const role = document.createElement('div');
  role.className = 'gcv-role';
  role.textContent = msg.role === 'user'
    ? gcvLabel('gcv_role_user', 'あなた')
    : gcvLabel('gcv_role_assistant', 'Grok');
  const copyBtn = document.createElement('button');
  copyBtn.type = 'button';
  copyBtn.className = 'gcv-btn gcv-msg-copy';
  copyBtn.textContent = '📋';
  copyBtn.title = gcvLabel('gcv_copy', 'コピー');
  copyBtn.setAttribute('aria-label', gcvLabel('gcv_copy', 'コピー'));
  copyBtn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    const body = String(msg.text || '');
    if (!body) return;
    navigator.clipboard.writeText(body).then(() => {
      const prev = copyBtn.textContent;
      copyBtn.textContent = '✓';
      copyBtn.classList.add('copied');
      showToast(gcvLabel('gcv_copied', 'コピーしました'));
      setTimeout(() => {
        copyBtn.textContent = prev;
        copyBtn.classList.remove('copied');
      }, 1000);
    }).catch(() => {
      showToast(gcvLabel('gcv_copy_failed', 'コピーに失敗しました'));
    });
  });
  head.appendChild(role);
  head.appendChild(copyBtn);
  const text = document.createElement('div');
  text.className = 'gcv-text';
  gcvFillTextWithLinks(text, msg.text, sessionId);
  item.appendChild(head);
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
    for (const m of msgs) list.appendChild(gcvRenderMessage(m, sid));
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
  const sid = gcvState.sid;
  try {
    const limit = Math.min(GCV_PAGE_MESSAGES, gcvState.offset);
    const offset = gcvState.offset - limit;
    const resp = await gcvFetchPage(sid, offset, limit);
    if (!gcvState) return;
    gcvState.total = resp.total || gcvState.total;
    gcvState.offset = resp.offset ?? offset;
    const list = gcvRoot ? gcvRoot.querySelector('.gcv-list') : null;
    const body = gcvRoot ? gcvRoot.querySelector('.gcv-body') : null;
    if (!list || !body) return;
    // 先頭に差し込んでもスクロール位置（見えている行）が動かないよう補正する
    const prevHeight = body.scrollHeight;
    const frag = document.createDocumentFragment();
    for (const m of (resp.messages || [])) frag.appendChild(gcvRenderMessage(m, sid));
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

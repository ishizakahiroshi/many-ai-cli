// mobile-terminal-lite.ts
// スマホ幅で xterm.js のフル描画を隠し、Web UI 送信イベントと scanBuffer(activeSessionId)
// のクリーンテキスト差分からチャットトランスクリプトを表示するモバイルビュー。
// xterm.js 自身は背後で生きているため、承認検出・PTY 受信・スクロールバック保持は
// すべてそのまま動く。本ビューは「見せ方の置き換え」だけで、データ経路には触れない。

import { t } from '../i18n.js';
// 切り分け用の計測（instrumentation.json の mobile-lite-empty）。?mtldebug=1 のときだけ動く。
import './debug-mobile-view.js';
import { activeSessionId } from './state.js';
import { getMobileTranscriptMessages, mobileTranscriptStatusText, syncMobileTranscriptFromBuffer } from './mobile-transcript.js';
import { scanBuffer } from './terminal.js';

// スマホ幅判定（モジュール内のみ使用）。
const _mtlMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)') : null;
function isMobileViewport(): boolean { return !!_mtlMql?.matches; }

let _refreshTimer: number | null = null;
const lastRenderedMessageId = new Map<number, number>();

function getOrCreateLiteContainer(): HTMLElement | null {
  let el = document.getElementById('mobile-terminal-lite');
  if (el) return el;
  const wrapper = document.getElementById('terminal-wrapper');
  if (!wrapper) return null;

  el = document.createElement('div');
  el.id = 'mobile-terminal-lite';

  // C4: 「詳細を見る (全画面)」を中央配置の幅広ボタンから、ターミナル右上の小さなアイコンに変更。
  // ターミナル本文と入力欄の間にあった大きなボタン帯を撤去して視線の流れを途切れさせない。
  const detailBtn = document.createElement('button');
  detailBtn.id = 'mobile-terminal-lite-detail';
  detailBtn.type = 'button';
  detailBtn.className = 'mtl-detail-icon';
  detailBtn.textContent = '⛶';
  detailBtn.title = t('mobile_terminal_lite_show_detail');
  detailBtn.setAttribute('aria-label', t('mobile_terminal_lite_show_detail'));
  detailBtn.addEventListener('click', openDetailModal);
  el.appendChild(detailBtn);

  const chat = document.createElement('div');
  chat.id = 'mobile-chat-view';
  chat.className = 'mtl-chat-view';

  const messages = document.createElement('div');
  messages.className = 'mtl-chat-messages';
  chat.appendChild(messages);

  const status = document.createElement('div');
  status.className = 'mtl-chat-status';
  status.innerHTML = '<span class="mtl-chat-status-dot" aria-hidden="true"></span><span class="mtl-chat-status-text"></span>';
  chat.appendChild(status);

  const jump = document.createElement('button');
  jump.type = 'button';
  jump.className = 'mtl-jump-latest';
  jump.textContent = t('mobile_chat_jump_latest');
  jump.hidden = true;
  jump.addEventListener('click', () => {
    chat.scrollTop = chat.scrollHeight;
    jump.hidden = true;
  });
  chat.appendChild(jump);

  chat.addEventListener('scroll', () => {
    const distance = chat.scrollHeight - chat.scrollTop - chat.clientHeight;
    jump.hidden = distance < 80;
  }, { passive: true });
  el.appendChild(chat);

  // terminal-area-wrapper の直後に挿入（同じ階層）。CSS で個別セッション + mobile 幅時のみ表示。
  wrapper.appendChild(el);
  return el;
}

// PTY バッファのクリーンテキスト差分をチャットビューへ反映する。1 秒ごとの定期更新と、
// activateSession 直後の明示呼び出しから入る。
export function refreshMobileTerminalLite(): void {
  if (!isMobileViewport()) return;
  if (activeSessionId === null) return;
  const el = getOrCreateLiteContainer();
  if (!el) return;
  syncMobileTranscriptFromBuffer(activeSessionId);
  renderChatView(el, activeSessionId);
}

function renderChatView(root: HTMLElement, sessionId: number): void {
  const chat = root.querySelector<HTMLElement>('#mobile-chat-view');
  const list = root.querySelector<HTMLElement>('.mtl-chat-messages');
  const status = root.querySelector<HTMLElement>('.mtl-chat-status');
  const statusText = root.querySelector<HTMLElement>('.mtl-chat-status-text');
  const jump = root.querySelector<HTMLButtonElement>('.mtl-jump-latest');
  if (!chat || !list || !status || !statusText) return;

  const previousSession = Number(root.dataset.sessionId || 0);
  if (previousSession !== sessionId) {
    root.dataset.sessionId = String(sessionId);
    list.replaceChildren();
    lastRenderedMessageId.set(sessionId, 0);
  }

  const distance = chat.scrollHeight - chat.scrollTop - chat.clientHeight;
  const shouldFollow = distance < 80 || list.childElementCount === 0;
  const messages = getMobileTranscriptMessages(sessionId);
  const renderedId = lastRenderedMessageId.get(sessionId) || 0;

  const validIds = new Set(messages.map((msg) => String(msg.id)));
  list.querySelectorAll<HTMLElement>('.mtl-msg').forEach((el) => {
    const id = el.dataset.msgId || '';
    if (!validIds.has(id)) el.remove();
  });

  for (const msg of messages) {
    if (msg.id <= renderedId && list.querySelector(`.mtl-msg[data-msg-id="${msg.id}"]`)) continue;
    const item = document.createElement('div');
    item.className = `mtl-msg mtl-msg-${msg.role}`;
    item.dataset.msgId = String(msg.id);
    item.textContent = msg.text;
    list.appendChild(item);
  }
  lastRenderedMessageId.set(sessionId, messages.length > 0 ? messages[messages.length - 1].id : 0);

  const last = messages[messages.length - 1];
  if (last && last.role === 'ai') {
    const lastEl = list.querySelector<HTMLElement>(`.mtl-msg[data-msg-id="${last.id}"]`);
    if (lastEl && lastEl.textContent !== last.text) lastEl.textContent = last.text;
  }

  statusText.textContent = mobileTranscriptStatusText(sessionId);
  list.appendChild(status);
  if (shouldFollow) {
    chat.scrollTop = chat.scrollHeight;
    if (jump) jump.hidden = true;
  } else if (jump) {
    jump.hidden = false;
  }
}

// 「詳細を見る」モーダル: scanBuffer 全行を pre で全画面表示する。
function openDetailModal(e?: Event): void {
  if (e) e.stopPropagation();
  if (activeSessionId === null) return;
  // 詳細モーダルは「全行を確認できる」が目的なので collapse は適用しない（生のログを保つ）。
  const allLines = scanBuffer(activeSessionId);
  while (allLines.length > 0 && allLines[allLines.length - 1].trim() === '') allLines.pop();

  const overlay = document.createElement('div');
  overlay.className = 'mtl-detail-overlay';

  const box = document.createElement('div');
  box.className = 'mtl-detail-box';

  const head = document.createElement('div');
  head.className = 'mtl-detail-head';

  const title = document.createElement('span');
  title.className = 'mtl-detail-title';
  title.textContent = t('mobile_terminal_lite_detail_title', { n: String(allLines.length) });
  head.appendChild(title);

  const tailBtn = document.createElement('button');
  tailBtn.type = 'button';
  tailBtn.className = 'mtl-detail-tail';
  tailBtn.textContent = t('mobile_terminal_lite_tail');
  tailBtn.addEventListener('click', () => { pre.scrollTop = pre.scrollHeight; });
  head.appendChild(tailBtn);

  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'mtl-detail-close';
  closeBtn.textContent = '✕';
  closeBtn.setAttribute('aria-label', t('settings_close'));
  head.appendChild(closeBtn);

  box.appendChild(head);

  const pre = document.createElement('pre');
  pre.className = 'mtl-detail-pre';
  pre.textContent = allLines.join('\n');
  box.appendChild(pre);

  let detailFontSize = 12.5;
  const setDetailFontSize = (next: number) => {
    detailFontSize = Math.max(10, Math.min(16, next));
    pre.style.fontSize = `${detailFontSize}px`;
  };
  let pinchStartDistance = 0;
  let pinchStartFontSize = detailFontSize;
  pre.addEventListener('touchstart', (ev) => {
    if (ev.touches.length !== 2) return;
    pinchStartDistance = Math.hypot(
      ev.touches[0].clientX - ev.touches[1].clientX,
      ev.touches[0].clientY - ev.touches[1].clientY,
    );
    pinchStartFontSize = detailFontSize;
  }, { passive: true });
  pre.addEventListener('touchmove', (ev) => {
    if (ev.touches.length !== 2 || pinchStartDistance <= 0) return;
    ev.preventDefault();
    const distance = Math.hypot(
      ev.touches[0].clientX - ev.touches[1].clientX,
      ev.touches[0].clientY - ev.touches[1].clientY,
    );
    setDetailFontSize(pinchStartFontSize * (distance / pinchStartDistance));
  }, { passive: false });

  overlay.appendChild(box);
  const close = () => {
    document.removeEventListener('keydown', onKey, true);
    overlay.remove();
  };
  overlay.addEventListener('click', (ev) => {
    if (ev.target === overlay) close();
  });
  function onKey(ev: KeyboardEvent): void {
    if (ev.key !== 'Escape') return;
    ev.preventDefault();
    ev.stopPropagation();
    close();
  }
  closeBtn.addEventListener('click', close);
  document.addEventListener('keydown', onKey, true);

  document.body.appendChild(overlay);
  // 末尾追従。
  setTimeout(() => { pre.scrollTop = pre.scrollHeight; }, 0);
}

function startRefresh(): void {
  if (_refreshTimer !== null) return;
  _refreshTimer = window.setInterval(() => {
    if (!isMobileViewport()) return;
    if (activeSessionId === null) return;
    refreshMobileTerminalLite();
  }, 1000);
}
startRefresh();

// 幅変化（スマホ↔PC）で初回描画が抜けないよう、change でも refresh する。
if (_mtlMql) {
  _mtlMql.addEventListener('change', () => {
    if (isMobileViewport()) refreshMobileTerminalLite();
  });
}

// 他モジュール（activateSession 後の即時 refresh 用）から呼べるよう公開。
(window as any).refreshMobileTerminalLite = refreshMobileTerminalLite;

export function clearMobileTerminalLiteSession(sessionId: number): void {
  lastRenderedMessageId.delete(sessionId);
}

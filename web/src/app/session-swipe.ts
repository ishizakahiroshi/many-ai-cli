// session-swipe.ts
// C4: セッション切替スワイプ（ターミナル上部ハンドル帯での左右スワイプ）
// #swipe-session-handle 要素でジェスチャを検出し、アクティブセッションを前後に切替える。
// xterm.js の領域とは分離された専用帯なので xterm のタッチスクロールとは干渉しない。

import { t } from '../i18n.js';
import { activeSessionId, orderSessions } from './state.js';
import { activateSession } from './session-list.js';
import { showToast } from './util.js';

const SWIPE_THRESHOLD = 60; // px
const SWIPE_AXIS_LOCK = 10; // px

function sessionTitle(s: any): string {
  if (!s) return '?';
  return s.label || s.last_message || s.first_message
    || (s.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop()
    || `#${s.id}`;
}

function initSessionSwipe() {
  const handle = document.getElementById('swipe-session-handle');
  if (!handle) return;

  let startX = 0;
  let startY = 0;
  let currentX = 0;
  let axisLocked: 'h' | 'v' | null = null;
  let dragging = false;

  handle.addEventListener('pointerdown', (e: Event) => {
    const pe = e as PointerEvent;
    startX = pe.clientX;
    startY = pe.clientY;
    currentX = pe.clientX;
    axisLocked = null;
    dragging = false;
    handle.setPointerCapture(pe.pointerId);
  }, { passive: true });

  handle.addEventListener('pointermove', (e: Event) => {
    const pe = e as PointerEvent;
    currentX = pe.clientX;
    const dx = currentX - startX;
    const dy = pe.clientY - startY;

    if (axisLocked === null) {
      if (Math.abs(dx) < SWIPE_AXIS_LOCK && Math.abs(dy) < SWIPE_AXIS_LOCK) return;
      axisLocked = Math.abs(dx) >= Math.abs(dy) ? 'h' : 'v';
    }

    if (axisLocked === 'h') {
      dragging = true;
      pe.preventDefault();
    }
  }, { passive: false });

  handle.addEventListener('pointerup', (e: Event) => {
    const pe = e as PointerEvent;
    if (!dragging) { axisLocked = null; return; }
    dragging = false;
    axisLocked = null;

    const dx = currentX - startX;
    if (Math.abs(dx) < SWIPE_THRESHOLD) return;

    const sessions = orderSessions();
    if (!sessions || sessions.length < 2) return;

    const currentIdx = sessions.findIndex(s => s && s.id === activeSessionId);
    if (currentIdx < 0) return;

    // 右スワイプ = 前のセッション / 左スワイプ = 次のセッション
    let nextIdx: number;
    if (dx > 0) {
      nextIdx = (currentIdx - 1 + sessions.length) % sessions.length;
    } else {
      nextIdx = (currentIdx + 1) % sessions.length;
    }

    const nextSession = sessions[nextIdx];
    if (!nextSession || nextSession.id === activeSessionId) return;

    activateSession(nextSession.id);

    // セッション名トースト
    const name = sessionTitle(nextSession);
    const msg = (t('session_switch_toast') || 'Session: {name}').replace('{name}', `#${nextSession.id} ${name}`);
    showToast(msg);
  }, { passive: true });

  handle.addEventListener('pointercancel', () => {
    dragging = false;
    axisLocked = null;
  }, { passive: true });
}

// DOM 準備後に初期化
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initSessionSwipe);
} else {
  queueMicrotask(initSessionSwipe);
}

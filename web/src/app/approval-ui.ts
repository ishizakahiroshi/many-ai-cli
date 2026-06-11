// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { actionBarShownAt, activeSessionId, approvalRawOptionsCache, approvalSourceCache, approvalVisibleCache, enqueueApprovalAutoSwitch, lastActionBarRender, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, multiSelectSelections, removeApprovalAutoSwitchTarget, set_actionBarFocusIdx, set_batchFocusIdx, set_multiSelectFocusIdx } from './state.js';
import { playNotificationSound, showDesktopApprovalNotification } from './settings.js';
import { ws } from './ws-client.js';
import { showActionBar } from './approval.js';

// UI/cache adapter for approval detection. Parser code must not depend on this.
(function (root) {
  'use strict';

  function notifyApprovalQueue() {
    try {
      root.dispatchEvent(new CustomEvent('approval-queue-updated'));
    } catch (_) {}
  }

  function sendSessionHint(id, visible) {
    ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: !!visible }));
  }

  function setApprovalVisible(id, visible, options: any = {}) {
    const next = !!visible;
    const wasVisible = !!approvalVisibleCache.get(id);
    if (wasVisible === next && !options.forceNotify) return wasVisible;
    approvalVisibleCache.set(id, next);
    if (next) {
      if (options.autoSwitch !== false) enqueueApprovalAutoSwitch(id);
      if (options.sound) playNotificationSound();
      showDesktopApprovalNotification(id);
    } else {
      removeApprovalAutoSwitchTarget(id);
    }
    sendSessionHint(id, next);
    notifyApprovalQueue();
    return wasVisible;
  }

  function cacheApprovalOptions(id, options) {
    approvalRawOptionsCache.set(id, options);
    const source = Array.isArray(options) && options[0] && options[0]._approvalSource;
    if (source !== 'go_vt') approvalSourceCache.delete(id);
    notifyApprovalQueue();
  }

  function clearApprovalOptions(id) {
    approvalRawOptionsCache.delete(id);
    approvalSourceCache.delete(id);
    notifyApprovalQueue();
  }

  function showOptions(bar, id, options, forceStickToBottom = false) {
    cacheApprovalOptions(id, options);
    showActionBar(bar, id, options, forceStickToBottom);
  }

  function clearActionBarDom() {
    const bar = document.getElementById('action-bar');
    if (bar) {
      bar.classList.remove('visible', 'batch');
      bar.innerHTML = '';
    }
    lastActionBarRender.sessionId = null;
    lastActionBarRender.sig = null;
    set_actionBarFocusIdx(-1);
    set_batchFocusIdx(-1);
    set_multiSelectFocusIdx(-1);
    if (activeSessionId != null) { actionBarShownAt.delete(activeSessionId); multiSelectSelections.delete(activeSessionId); }
    notifyApprovalQueue();
  }

  function setMultiQuestionBannerVisible(visible) {
    const banner = document.getElementById('multi-question-banner');
    if (!banner) return;
    if (visible) {
      banner.innerHTML = '';
      const msg = document.createElement('span');
      msg.className = 'multi-question-banner-text';
      msg.textContent = t('multi_question_banner');
      const closeBtn = document.createElement('button');
      closeBtn.type = 'button';
      closeBtn.className = 'multi-question-banner-close';
      closeBtn.textContent = '✕';
      closeBtn.title = t('multi_question_banner_close_tooltip') || 'Dismiss';
      closeBtn.addEventListener('click', () => {
        const id = activeSessionId;
        if (!id) { banner.hidden = true; return; }
        multiQuestionDismissedCache.set(id, true);
        multiQuestionVisibleCache.delete(id);
        multiQuestionLatchAt.delete(id);
        banner.hidden = true;
        if (approvalVisibleCache.get(id) && !(approvalRawOptionsCache.get(id)?.length > 0)) {
          setApprovalVisible(id, false);
        }
      });
      banner.appendChild(msg);
      banner.appendChild(closeBtn);
      banner.hidden = false;
    } else {
      banner.hidden = true;
    }
  }

  // 承認可視ヒントの定期再主張: Hub 側の approvalVisible はリース制
  // （server.go の approvalVisibleLease = 15s）のため、可視中はリースの 1/3 間隔で
  // true を再送して維持する。false ヒントがどの経路で失われても（リロード desync・
  // 複数クライアント・H9 復元固着等）再主張が止まれば Hub 側がリース切れで自動回復する。
  const APPROVAL_HINT_REASSERT_MS = 5000; // approvalVisibleLease(15s) の 1/3

  function reassertApprovalHints() {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    approvalVisibleCache.forEach((visible, id) => {
      if (visible) sendSessionHint(id, true);
    });
  }
  setInterval(reassertApprovalHints, APPROVAL_HINT_REASSERT_MS);

  const api = {
    sendSessionHint,
    setApprovalVisible,
    cacheApprovalOptions,
    clearApprovalOptions,
    showOptions,
    clearActionBarDom,
    setMultiQuestionBannerVisible,
    reassertApprovalHints,
  };

  root.approvalUiAdapter = api;
  root.setMultiQuestionBannerVisible = setMultiQuestionBannerVisible;
})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval UI adapter (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalUiAdapter = __esmRoot.approvalUiAdapter;
export const setMultiQuestionBannerVisible = __esmRoot.setMultiQuestionBannerVisible;

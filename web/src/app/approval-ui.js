// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { actionBarShownAt, activeSessionId, approvalRawOptionsCache, approvalSourceCache, approvalVisibleCache, enqueueApprovalAutoSwitch, lastActionBarRender, multiQuestionDismissedCache, multiQuestionVisibleCache, removeApprovalAutoSwitchTarget, set_actionBarFocusIdx, set_batchFocusIdx } from './state.js';
import { playNotificationSound } from './settings.js';
import { ws } from './ws-client.js';
import { showActionBar } from './approval.js';

// UI/cache adapter for approval detection. Parser code must not depend on this.
(function (root) {
  'use strict';

  function sendSessionHint(id, visible) {
    ws.send(JSON.stringify({ type: 'session_hint', session_id: id, approval_visible: !!visible }));
  }

  function setApprovalVisible(id, visible, options = {}) {
    const next = !!visible;
    const wasVisible = !!approvalVisibleCache.get(id);
    if (wasVisible === next && !options.forceNotify) return wasVisible;
    approvalVisibleCache.set(id, next);
    if (next) {
      if (options.autoSwitch !== false) enqueueApprovalAutoSwitch(id);
      if (options.sound) playNotificationSound();
    } else {
      removeApprovalAutoSwitchTarget(id);
    }
    sendSessionHint(id, next);
    return wasVisible;
  }

  function cacheApprovalOptions(id, options) {
    approvalRawOptionsCache.set(id, options);
    const source = Array.isArray(options) && options[0] && options[0]._approvalSource;
    if (source !== 'go_vt') approvalSourceCache.delete(id);
  }

  function clearApprovalOptions(id) {
    approvalRawOptionsCache.delete(id);
    approvalSourceCache.delete(id);
  }

  function showOptions(bar, id, options, showExpand = false, forceStickToBottom = false) {
    cacheApprovalOptions(id, options);
    showActionBar(bar, id, options, showExpand, forceStickToBottom);
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
    if (activeSessionId != null) actionBarShownAt.delete(activeSessionId);
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

  const api = {
    sendSessionHint,
    setApprovalVisible,
    cacheApprovalOptions,
    clearApprovalOptions,
    showOptions,
    clearActionBarDom,
    setMultiQuestionBannerVisible,
  };

  root.approvalUiAdapter = api;
  root.setMultiQuestionBannerVisible = setMultiQuestionBannerVisible;
})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval UI adapter (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalUiAdapter = __esmRoot.approvalUiAdapter;
export const setMultiQuestionBannerVisible = __esmRoot.setMultiQuestionBannerVisible;

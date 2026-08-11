// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { actionBarShownAt, activeSessionId, approvalCandidateDebugKey, approvalCandidateIdentity, approvalRawOptionsCache, approvalSourceCache, approvalSuppressedCache, approvalSuppressedDismissedCache, approvalVisibleCache, enqueueApprovalAutoSwitch, lastActionBarRender, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, multiSelectSelections, removeApprovalAutoSwitchTarget, set_actionBarFocusIdx, set_batchFocusIdx, set_multiSelectFocusIdx } from './state.js';
import { playNotificationSound, showDesktopApprovalNotification } from './settings.js';
import { ws } from './ws-client.js';
import { reshowActionBar, showActionBar } from './approval.js';
import { inheritMarkerBlockSig } from './approval-parser.js';
import { suppressPtyResizeForInputLayout, syncPtySizeToViewportAfterLayout } from './terminal.js';

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
    inheritMarkerBlockSig(options, approvalRawOptionsCache.get(id));
    approvalRawOptionsCache.set(id, options);
    const source = Array.isArray(options) && options[0] && options[0]._approvalSource;
    const hasMarkerIdentity = !!(Array.isArray(options) && options[0] && options[0]._blockSig);
    // 同一マーカーを fallback parser が再抽出した cache 更新では hub_marker provenance を
    // 保持する。これを消すと回答時の恒久抑止と Hub push の重複防止が同時に外れる。
    if (source !== 'go_vt' && !hasMarkerIdentity) approvalSourceCache.delete(id);
    notifyApprovalQueue();
  }

  function clearApprovalOptions(id) {
    approvalRawOptionsCache.delete(id);
    approvalSourceCache.delete(id);
    notifyApprovalQueue();
  }

  // そのセッションの承認パネルが実際に画面へ出ているか。
  // 抑止告知バナーの目的は「承認待ちなのに画面へ何も出ない」状態の説明なので、
  // パネルが出ているセッションで告知すると文言と画面が矛盾する。
  // hideActionBar は approvalVisibleCache を false にし approvalRawOptionsCache も
  // 落とすため、この 2 つの AND が「今この瞬間パネルが出ている」の代理指標になる。
  function approvalPanelVisibleFor(id) {
    if (id == null || !approvalVisibleCache.get(id)) return false;
    const opts = approvalRawOptionsCache.get(id);
    return Array.isArray(opts) && opts.length > 0;
  }

  function showOptions(bar, id, options, forceStickToBottom = false) {
    cacheApprovalOptions(id, options);
    const identity = approvalCandidateIdentity(id, options, approvalSourceCache.get(id)?.source === 'go_vt' ? 'native' : 'marker');
    const sameVisibleCandidate = !!(bar && bar.classList.contains('visible') &&
      bar.children.length > 0 && bar.dataset.approvalSessionId === String(id) &&
      bar.dataset.approvalCandidateKey === identity.candidateKey &&
      bar.dataset.approvalSourceEpoch === String(identity.sourceEpoch));
    if (sameVisibleCandidate) {
      return;
    }
    // showActionBar は手動 dismiss 中などに何も描かず return する。描画が最後まで
    // 通った時だけ 3 箇所の描画完了地点が actionBarShownAt を更新するので、
    // その変化を「実際にパネルが出た」証拠として使う。
    const shownBefore = actionBarShownAt.get(id);
    showActionBar(bar, id, options, forceStickToBottom);
    // パネルが出た＝Hub 経路が壊れていてもクライアント再構成で拾えたということなので、
    // 残っている抑止告知は取り下げる。ここで cache ごと消さないと、回答してパネルが
    // 閉じた後のセッション切替で古い告知が復活する。
    if (actionBarShownAt.get(id) !== shownBefore) clearApprovalMarkerSuppressed(id);
  }

  function clearActionBarDom() {
    const bar = document.getElementById('action-bar');
    const wasVisible = !!(bar && bar.classList.contains('visible'));
    const clearId = activeSessionId;
    const clearOptions = clearId != null ? approvalRawOptionsCache.get(clearId) : null;
    const clearIdentity = clearId != null && Array.isArray(clearOptions) && clearOptions.length > 0
      ? approvalCandidateIdentity(clearId, clearOptions, approvalSourceCache.get(clearId)?.source === 'go_vt' ? 'native' : 'marker')
      : null;
    if (bar) {
      bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs');
      bar.innerHTML = '';
      delete bar.dataset.approvalSessionId;
      delete bar.dataset.approvalCandidateKey;
      delete bar.dataset.approvalSourceEpoch;
    }
    lastActionBarRender.sessionId = null;
    lastActionBarRender.sig = null;
    if (wasVisible && clearId != null) {
      suppressPtyResizeForInputLayout(350);
      syncPtySizeToViewportAfterLayout(clearId, true, 400, 'settle-after-clear', clearIdentity);
    }
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

  // 破損した承認マーカーブロックを抑止したことの告知バナー。
  // 抑止自体は Hub / クライアント双方で行うが、無音で捨てると「承認待ちのまま
  // 何も出ない」状態になり原因が分からない（bugfix_codex-approval-marker-vt-wrap-corruption）。
  // バナー DOM は全セッション共有の 1 個なので、表示内容はセッション別 cache に持つ。
  function approvalSuppressedReasonLabel(reason) {
    const key = 'approval_suppressed_reason_' + String(reason || '');
    const label = t(key);
    // 未知の reason（将来 Hub 側に分類が増えた場合）はキー名を素で出さない。
    return label && label !== key ? label : t('approval_suppressed_reason_client_corrupt');
  }

  // このバナーは通常フローの要素（approval.css の flex-shrink: 0）なので、出し入れの
  // たびに #terminal-area の高さが変わり、terminal.ts の ResizeObserver が rAF ごとに
  // sendResize を撃つ。SIGWINCH を受けた TUI は画面全体を描き直すため、Hub の VT ミラーが
  // 実端末と乖離しやすくなり、その乖離が次の「壊れた承認ブロック」を生んで再びこのバナーを
  // 呼ぶ増幅ループになる（2026-08-02 実測: rows が 10〜35 の間を秒間数回変動していた）。
  // 入力欄の伸縮と同じ扱いにして、連発を止めてからレイアウト確定後に実寸を 1 回だけ送る。
  function settleTerminalAfterBannerLayout() {
    if (activeSessionId === null) return;
    suppressPtyResizeForInputLayout(350);
    syncPtySizeToViewportAfterLayout(activeSessionId);
  }

  function renderApprovalSuppressedBannerFor(id) {
    const banner = document.getElementById('approval-suppressed-banner');
    if (!banner) return;
    const wasHidden = banner.hidden;
    const reason = id == null ? undefined : approvalSuppressedCache.get(id);
    // パネルが出ているセッションでは告知しない（セッション切替でこの経路から
    // 古い理由が描かれるのを防ぐ。noteApprovalMarkerSuppressed 側と同じ判定）。
    if (!reason || approvalSuppressedDismissedCache.get(id) || approvalPanelVisibleFor(id)) {
      banner.hidden = true;
      banner.innerHTML = '';
      if (!wasHidden) settleTerminalAfterBannerLayout();
      return;
    }
    banner.innerHTML = '';
    const msg = document.createElement('span');
    msg.className = 'multi-question-banner-text';
    msg.textContent = t('approval_suppressed_banner').replace('{reason}', approvalSuppressedReasonLabel(reason));
    const retryBtn = document.createElement('button');
    retryBtn.type = 'button';
    retryBtn.className = 'multi-question-banner-action';
    retryBtn.textContent = t('approval_suppressed_banner_retry');
    retryBtn.title = t('approval_suppressed_banner_retry_tooltip');
    retryBtn.addEventListener('click', () => {
      // ブラウザ側 xterm バッファを読み直す（Hub の VT ミラーとは独立した再構成なので、
      // Hub 側だけが壊れていた場合はここで拾い直せる）。
      reshowActionBar(id);
    });
    const closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.className = 'multi-question-banner-close';
    closeBtn.textContent = '✕';
    closeBtn.title = t('approval_suppressed_banner_close_tooltip') || 'Dismiss';
    closeBtn.addEventListener('click', () => {
      if (id != null) approvalSuppressedDismissedCache.set(id, true);
      // 直接 hidden を触らず render 経由にして、レイアウト確定処理を 1 箇所へ集約する。
      renderApprovalSuppressedBannerFor(id);
    });
    banner.appendChild(msg);
    banner.appendChild(retryBtn);
    banner.appendChild(closeBtn);
    banner.hidden = false;
    if (wasHidden) settleTerminalAfterBannerLayout();
  }

  // 新しい抑止イベント。✕ で閉じた状態は解除する（別事象なので出し直す）。
  function noteApprovalMarkerSuppressed(id, reason) {
    if (id == null) return;
    // 既にパネルが出ているなら Hub 経路の破損は実害が無い（クライアントが自分の
    // xterm バッファから同じ承認を再構成できていて、その再構成も
    // isCorruptHubMarkerOptions を通過している）。cache に積まず捨てる。
    // 積んでしまうと、回答してパネルが閉じた後のセッション切替で復活する。
    if (approvalPanelVisibleFor(id)) return;
    approvalSuppressedCache.set(id, String(reason || 'client_corrupt'));
    approvalSuppressedDismissedCache.delete(id);
    if (id === activeSessionId) renderApprovalSuppressedBannerFor(id);
  }

  // 正常な承認が出た・セッションが終わった等で告知を取り下げる。
  function clearApprovalMarkerSuppressed(id) {
    if (id == null) return;
    const had = approvalSuppressedCache.delete(id);
    approvalSuppressedDismissedCache.delete(id);
    if (had && id === activeSessionId) renderApprovalSuppressedBannerFor(id);
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
    renderApprovalSuppressedBannerFor,
    noteApprovalMarkerSuppressed,
    clearApprovalMarkerSuppressed,
    reassertApprovalHints,
  };

  root.approvalUiAdapter = api;
  root.setMultiQuestionBannerVisible = setMultiQuestionBannerVisible;
  root.renderApprovalSuppressedBannerFor = renderApprovalSuppressedBannerFor;
  root.noteApprovalMarkerSuppressed = noteApprovalMarkerSuppressed;
  root.clearApprovalMarkerSuppressed = clearApprovalMarkerSuppressed;
})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval UI adapter (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalUiAdapter = __esmRoot.approvalUiAdapter;
export const setMultiQuestionBannerVisible = __esmRoot.setMultiQuestionBannerVisible;
export const renderApprovalSuppressedBannerFor = __esmRoot.renderApprovalSuppressedBannerFor;
export const noteApprovalMarkerSuppressed = __esmRoot.noteApprovalMarkerSuppressed;
export const clearApprovalMarkerSuppressed = __esmRoot.clearApprovalMarkerSuppressed;

// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { actionBarShownAt, activeSessionId, approvalSuppressedCache, approvalSuppressedDismissedCache, isStaleHistoryRepaint } from './state.js';
import { displayedApprovalIdentity, foldApprovalPanel, requestApprovalResync, showActionBar } from './approval.js';
import { isTerminalShowingHistory, suppressPtyResizeForInputLayout, syncPtySizeToViewportAfterLayout } from './terminal.js';
import { probe } from '../debug/probe.js';

// 承認パネルの DOM 側の手続き（描く・片付ける・告知バナー）。承認の有無は Hub の記録が決める
// （approval-store.ts）。Parser code must not depend on this.
(function (root) {
  'use strict';

  // そのセッションの承認パネルが実際に画面へ出ているか。
  // 抑止告知バナーの目的は「承認待ちなのに画面へ何も出ない」状態の説明なので、
  // パネルが出ているセッションで告知すると文言と画面が矛盾する。
  // #action-bar は全セッション共有の 1 個で、描画時に描画元のセッション番号を焼く
  // （approval-owner.ts）。表示中で、焼かれた番号がそのセッションなら出ている。
  function approvalPanelVisibleFor(id) {
    if (id == null) return false;
    const bar = document.getElementById('action-bar');
    return !!(bar && bar.classList.contains('visible') && bar.children.length > 0 &&
      bar.dataset.approvalSessionId === String(id));
  }

  function showOptions(bar, id, options, forceStickToBottom = false) {
    const identity = displayedApprovalIdentity(id, options);
    const sameVisibleCandidate = !!(bar && bar.classList.contains('visible') &&
      bar.children.length > 0 && bar.dataset.approvalSessionId === String(id) &&
      bar.dataset.approvalCandidateKey === identity.candidateKey &&
      bar.dataset.approvalSourceEpoch === String(identity.sourceEpoch));
    // ページ送りで CLI が過去の画面を描いている間に、回答済みの本文が描き直された分。
    //
    // 代替画面バッファの provider ではホイールが PgUp として CLI へ届き、CLI 自身が
    // 過去の位置を描き直す。Hub の承認検出は VT ミラー＝今の画面を読むので、遡って
    // 読んでいるだけで回答済みの承認が「新しい候補」として届く（実測は docs/local/
    // bugfix_approval-bar-stale-options-scroll-mismatch_2026-08-19.md の 2026-08-23 追記。
    // 7 時間前に回答した承認ブロックがそのまま届いていた）。
    //
    // 条件を「回答済みの shape」に絞るのが要点。世代を問わず一度でも答えた中身だけを
    // 落とすので、遡っている最中に届いた未回答の承認は今までどおり出る。ページ計上は
    // CLI 内部のスクロール量ではなく送った鍵数の近似で、ライブへ戻ったことを取りこぼす
    // ことがあるため、遡り中の候補を一律に落とすと新しい承認を握り潰す（F-12 の再発）。
    // 判定式は approval-answered.ts の isStaleHistoryRepaint に集約した。
    const staleHistoryRepaint = isStaleHistoryRepaint(id, identity.shape, isTerminalShowingHistory(id));
    probe('approval.data', () => ({ sessionId: id, identity, options, skipped: sameVisibleCandidate || staleHistoryRepaint }));
    if (sameVisibleCandidate || staleHistoryRepaint) {
      return;
    }
    // showActionBar は帯に畳んだ記録などで何も描かず return する。描画が最後まで
    // 通った時だけ 3 箇所の描画完了地点が actionBarShownAt を更新するので、
    // その変化を「実際にパネルが出た」証拠として使う。
    const shownBefore = actionBarShownAt.get(id);
    showActionBar(bar, id, options, forceStickToBottom);
    // パネルが出た＝記録を読めたので、残っている抑止告知は取り下げる。ここで cache ごと
    // 消さないと、回答してパネルが閉じた後のセッション切替で古い告知が復活する。
    if (actionBarShownAt.get(id) !== shownBefore) clearApprovalMarkerSuppressed(id);
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
      closeBtn.setAttribute('aria-label', closeBtn.title);
      closeBtn.addEventListener('click', () => {
        // パネルの ✕ と同じく、告知の記録を 1 行の帯に畳む（帯を押すと告知が戻る）。
        // 承認があるかどうかは Hub の記録が決める（「保留中」は残る）。畳み状態は記録ごとに
        // approval-store.ts が持ち、記録が変われば捨てる。
        const id = activeSessionId;
        if (id !== null) foldApprovalPanel(id);
        else banner.hidden = true;
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
      // Hub へ問い直す。Hub が端末ミラーから候補を評価し直し、今の記録をこの画面へ送り直す。
      requestApprovalResync(id);
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
    // 既にパネルが出ているなら、描けている承認とは別の破損なので実害が無い。cache に積まず捨てる。
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

  const api = {
    showOptions,
    setMultiQuestionBannerVisible,
    renderApprovalSuppressedBannerFor,
    noteApprovalMarkerSuppressed,
    clearApprovalMarkerSuppressed,
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

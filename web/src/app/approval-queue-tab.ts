// approval-queue-tab.ts
// 承認キュー専用タブ: 全セッションの承認待ちをカード一覧で表示し、その場で回答する。
// スマホの「裁く」用途に最適化。ターミナル表示を経由せずに承認を完結させる。

import { t } from '../i18n.js';
import { activeSessionId, approvalRawOptionsCache, approvalVisibleCache, multiQuestionVisibleCache, orderSessions, sessions } from './state.js';
import { activateSession } from './session-list.js';
import { isBatchOptions } from './approval-parser.js';
import { sendChoice } from './approval.js';
import { showToast } from './util.js';

// 承認タブのバッジカウント更新
function getApprovalTabBadgeEl() {
  return document.getElementById('approval-tab-badge');
}

function pendingSessionIds() {
  return orderSessions()
    .filter(s => s && (approvalVisibleCache.get(s.id) || multiQuestionVisibleCache.get(s.id)))
    .map(s => s.id);
}

export function updateApprovalTabBadge() {
  const badge = getApprovalTabBadgeEl();
  if (!badge) return;
  const ids = pendingSessionIds();
  const n = ids.length;
  badge.textContent = String(n);
  badge.hidden = (n === 0);

  // タブボタン自体にも has-pending クラスを付与してアニメーション
  const tabBtn = document.querySelector('#unified-tab-bar .view-tab[data-tab="approval"]');
  if (tabBtn) tabBtn.classList.toggle('has-pending', n > 0);
}

// プロバイダ色ドット HTML
function providerDotHtml(provider) {
  const p = String(provider || '').toLowerCase();
  return `<span class="aqt-provider-dot prov-shape ${p}" aria-hidden="true"></span>`;
}

// セッションのタイトル文字列
function sessionTitle(s) {
  if (!s) return 'Session';
  const base = s.label || s.last_message || s.first_message
    || (s.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop()
    || `#${s.id}`;
  return `${base}`;
}

// 単一承認の選択肢ボタン群を生成（options はフラット配列前提）
function renderOptionButtons(container, sessionId, options) {
  container.innerHTML = '';
  if (!Array.isArray(options) || options.length === 0) return;

  options.forEach((opt) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'aqt-option-btn';
    if (opt.isCurrent) btn.classList.add('is-current');

    const label = opt.label || opt.send_text || String(opt.num);
    btn.textContent = `${opt.num}. ${label}`;
    btn.dataset.sendText = opt.send_text || '';
    btn.dataset.optNum = String(opt.num);

    btn.addEventListener('click', () => {
      // 既存の sendChoice 経路を流用（approvalConsumedSig / hideActionBar / chatHistory 連動を含む）
      sendChoice(sessionId, opt.num);
      // 即時フィードバックとしてボタンを無効化
      container.querySelectorAll('.aqt-option-btn').forEach(b => {
        (b as HTMLButtonElement).disabled = true;
      });
      btn.classList.add('chosen');
    });

    container.appendChild(btn);
  });
}

// 承認パネル全体を描画
function renderApprovalPane() {
  const pane = document.getElementById('approval-pane');
  if (!pane) return;

  const ids = pendingSessionIds();

  pane.innerHTML = '';

  if (ids.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'aqt-empty';
    const icon = document.createElement('div');
    icon.className = 'aqt-empty-icon';
    icon.textContent = '✓';
    const msg = document.createElement('div');
    msg.className = 'aqt-empty-msg';
    msg.textContent = t('approval_tab_empty');
    empty.appendChild(icon);
    empty.appendChild(msg);
    pane.appendChild(empty);
    updateApprovalTabBadge();
    return;
  }

  for (const id of ids) {
    const s = sessions.get(id);
    const options = approvalRawOptionsCache.get(id);
    const isMultiQ = !!multiQuestionVisibleCache.get(id);
    const isActive = id === activeSessionId;

    const card = document.createElement('div');
    card.className = 'aqt-card' + (isActive ? ' is-active-session' : '');
    card.dataset.sessionId = String(id);

    // ── カードヘッダー ──
    const header = document.createElement('div');
    header.className = 'aqt-card-header';

    const dotHtml = s ? providerDotHtml(s.provider) : '';
    const titleEl = document.createElement('div');
    titleEl.className = 'aqt-card-title';
    titleEl.innerHTML = `${dotHtml}<span class="aqt-session-id">#${id}</span> <span class="aqt-session-name">${escapeHtml(sessionTitle(s))}</span>`;

    const goBtn = document.createElement('button');
    goBtn.type = 'button';
    goBtn.className = 'aqt-go-terminal-btn';
    goBtn.textContent = t('approval_tab_go_terminal');
    goBtn.title = t('approval_tab_go_terminal');
    goBtn.addEventListener('click', () => {
      activateSession(id);
      // ターミナルタブへ切替
      const termTab = document.querySelector('#unified-tab-bar .view-tab[data-tab="terminal"]') as HTMLButtonElement | null;
      termTab?.click();
    });

    header.appendChild(titleEl);
    header.appendChild(goBtn);
    card.appendChild(header);

    // ── 質問文 ──
    if (isMultiQ && (!options || options.length === 0)) {
      const qEl = document.createElement('div');
      qEl.className = 'aqt-question aqt-question--multiq';
      qEl.textContent = t('approval_tab_multiq_hint');
      card.appendChild(qEl);

      const goBtn2 = document.createElement('button');
      goBtn2.type = 'button';
      goBtn2.className = 'aqt-option-btn';
      goBtn2.textContent = t('approval_tab_go_terminal_full');
      goBtn2.addEventListener('click', () => {
        activateSession(id);
        const termTab = document.querySelector('#unified-tab-bar .view-tab[data-tab="terminal"]') as HTMLButtonElement | null;
        termTab?.click();
      });
      card.appendChild(goBtn2);
    } else if (options && options.length > 0) {
      if (isBatchOptions(options)) {
        // バッチ（複数質問）形式: ターミナルへ誘導
        const qEl = document.createElement('div');
        qEl.className = 'aqt-question aqt-question--multiq';
        qEl.textContent = t('approval_tab_multiq_hint');
        card.appendChild(qEl);

        const goBtn2 = document.createElement('button');
        goBtn2.type = 'button';
        goBtn2.className = 'aqt-option-btn';
        goBtn2.textContent = t('approval_tab_go_terminal_full');
        goBtn2.addEventListener('click', () => {
          activateSession(id);
          const termTab = document.querySelector('#unified-tab-bar .view-tab[data-tab="terminal"]') as HTMLButtonElement | null;
          termTab?.click();
        });
        card.appendChild(goBtn2);
      } else {
        // 通常承認: 選択肢ボタンを直接表示
        const btnArea = document.createElement('div');
        btnArea.className = 'aqt-options';
        renderOptionButtons(btnArea, id, options);
        card.appendChild(btnArea);

        // スワイプ承認: 選択肢が 1〜2 つのカードのみ対象
        if (options.length >= 1 && options.length <= 2) {
          attachSwipeGesture(card, id, options);
        }
      }
    } else {
      const qEl = document.createElement('div');
      qEl.className = 'aqt-question aqt-question--detecting';
      qEl.textContent = t('approval_tab_detecting');
      card.appendChild(qEl);
    }

    pane.appendChild(card);
  }

  updateApprovalTabBadge();
}

function escapeHtml(s) {
  return String(s || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ──────────────────────────────────────────
// C4: スワイプ承認（承認カード）
// ──────────────────────────────────────────

// スワイプ判定閾値 (px)
const SWIPE_THRESHOLD = 72;
// スワイプ開始後、縦方向の動きが横方向より大きければスクロール扱いにする
const SWIPE_AXIS_LOCK_THRESHOLD = 10;

// Undo トーストで使うタイマー管理
let _undoTimer: ReturnType<typeof setTimeout> | null = null;

function attachSwipeGesture(card: HTMLElement, sessionId: number, options: any[]) {
  // 右スワイプ = options[0] (許可系), 左スワイプ = options[1] or なし (拒否系)
  const acceptOpt = options[0];
  const rejectOpt = options.length >= 2 ? options[1] : null;

  // スワイプ中のオーバーレイ（背景色 + ラベル）
  const overlay = document.createElement('div');
  overlay.className = 'aqt-swipe-overlay';
  overlay.setAttribute('aria-hidden', 'true');
  const overlayLabel = document.createElement('span');
  overlayLabel.className = 'aqt-swipe-label';
  overlay.appendChild(overlayLabel);
  card.appendChild(overlay);

  let startX = 0;
  let startY = 0;
  let currentX = 0;
  let axisLocked: 'h' | 'v' | null = null;
  let dragging = false;
  let consumed = false;

  function resetCardPosition() {
    card.style.transform = '';
    card.style.transition = '';
    overlay.style.opacity = '0';
    overlay.className = 'aqt-swipe-overlay';
  }

  function onPointerDown(e: PointerEvent) {
    if (consumed) return;
    // ボタンタップは通常クリックに任せる
    if ((e.target as Element)?.closest('button')) return;
    startX = e.clientX;
    startY = e.clientY;
    currentX = e.clientX;
    axisLocked = null;
    dragging = false;
    card.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (consumed) return;
    currentX = e.clientX;
    const dx = currentX - startX;
    const dy = e.clientY - startY;

    if (axisLocked === null) {
      if (Math.abs(dx) < SWIPE_AXIS_LOCK_THRESHOLD && Math.abs(dy) < SWIPE_AXIS_LOCK_THRESHOLD) return;
      axisLocked = Math.abs(dx) >= Math.abs(dy) ? 'h' : 'v';
    }

    if (axisLocked === 'v') return;

    dragging = true;
    e.preventDefault();

    // 右スワイプのみ対象（拒否側がない場合は左も無効）
    const clampedDx = rejectOpt ? dx : Math.max(0, dx);
    card.style.transition = 'none';
    card.style.transform = `translateX(${clampedDx}px)`;

    const ratio = Math.min(1, Math.abs(clampedDx) / SWIPE_THRESHOLD);
    overlay.style.opacity = String(ratio * 0.85);

    if (clampedDx > 0) {
      overlay.className = 'aqt-swipe-overlay aqt-swipe-overlay--accept';
      overlayLabel.textContent = t('swipe_approve') || '許可';
    } else if (clampedDx < 0 && rejectOpt) {
      overlay.className = 'aqt-swipe-overlay aqt-swipe-overlay--reject';
      overlayLabel.textContent = t('swipe_reject') || '拒否';
    } else {
      overlay.style.opacity = '0';
    }
  }

  function onPointerUp(e: PointerEvent) {
    if (consumed || !dragging) {
      axisLocked = null;
      dragging = false;
      return;
    }
    dragging = false;
    axisLocked = null;

    const dx = currentX - startX;

    if (dx >= SWIPE_THRESHOLD) {
      // 右スワイプ → 許可
      consumed = true;
      executeSwipeAccept(card, sessionId, acceptOpt);
    } else if (dx <= -SWIPE_THRESHOLD && rejectOpt) {
      // 左スワイプ → 拒否（Undo トースト付き）
      consumed = true;
      executeSwipeRejectWithUndo(card, sessionId, rejectOpt);
    } else {
      // 閾値未満 → 元の位置へ戻す
      card.style.transition = 'transform 0.2s ease';
      resetCardPosition();
    }
  }

  function onPointerCancel() {
    if (!consumed) resetCardPosition();
    dragging = false;
    axisLocked = null;
  }

  card.addEventListener('pointerdown', onPointerDown as EventListener, { passive: true });
  card.addEventListener('pointermove', onPointerMove as EventListener, { passive: false });
  card.addEventListener('pointerup', onPointerUp as EventListener, { passive: true });
  card.addEventListener('pointercancel', onPointerCancel as EventListener, { passive: true });
}

function executeSwipeAccept(card: HTMLElement, sessionId: number, opt: any) {
  card.style.transition = 'transform 0.22s ease, opacity 0.22s ease';
  card.style.transform = `translateX(110%)`;
  (card as any).style.opacity = '0';
  setTimeout(() => {
    sendChoice(sessionId, opt.num);
  }, 180);
}

function executeSwipeRejectWithUndo(card: HTMLElement, sessionId: number, opt: any) {
  card.style.transition = 'transform 0.22s ease, opacity 0.22s ease';
  card.style.transform = `translateX(-110%)`;
  (card as any).style.opacity = '0';

  // 3 秒 Undo トースト
  if (_undoTimer) clearTimeout(_undoTimer);
  let undone = false;

  const undoLabel = t('swipe_undo') || '元に戻す';
  const rejectLabel = t('swipe_reject') || '拒否';
  const msg = `${rejectLabel} — ${undoLabel}`;
  showToast(msg);

  // トーストに Undo ボタンを追加
  const toastEl = document.getElementById('toast');
  if (toastEl) {
    const undoBtn = document.createElement('button');
    undoBtn.className = 'aqt-undo-toast-btn';
    undoBtn.textContent = undoLabel;
    undoBtn.addEventListener('click', () => {
      if (undone) return;
      undone = true;
      if (_undoTimer) { clearTimeout(_undoTimer); _undoTimer = null; }
      // カードを元の位置へ復元（再描画トリガー）
      card.style.transition = 'transform 0.22s ease, opacity 0.22s ease';
      card.style.transform = '';
      (card as any).style.opacity = '';
      toastEl.textContent = '';
      // consumed フラグは解除できないのでパネル全体を再描画して復元
      window.dispatchEvent(new CustomEvent('approval-queue-updated'));
    });
    toastEl.appendChild(undoBtn);
  }

  _undoTimer = setTimeout(() => {
    _undoTimer = null;
    if (!undone) {
      sendChoice(sessionId, opt.num);
    }
  }, 3000);
}

// 承認タブが表示中かどうか
function isApprovalTabVisible() {
  const pane = document.getElementById('approval-pane');
  const area = document.getElementById('display-area');
  if (!pane || !area) return false;
  return area.classList.contains('mode-approval') && !area.hidden;
}

// 外部から呼ばれる更新関数
export function refreshApprovalTab() {
  updateApprovalTabBadge();
  if (isApprovalTabVisible()) {
    renderApprovalPane();
  }
}

// タブが切り替えられて承認タブが開いたときに描画
window.addEventListener('session-view-mode-changed', (e: any) => {
  if (e?.detail?.name === 'approval') {
    renderApprovalPane();
  }
});

// 承認キャッシュが変わるたびに呼ばれる既存フック
// approval-queue.ts の updateApprovalQueue と同様に window イベントで連動
window.addEventListener('approval-queue-updated', () => {
  updateApprovalTabBadge();
  if (isApprovalTabVisible()) {
    renderApprovalPane();
  }
});

// 初期バッジ設定（DOM 準備後）
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => updateApprovalTabBadge());
} else {
  queueMicrotask(updateApprovalTabBadge);
}

// 定期バッジ更新（1 秒間隔）
setInterval(() => {
  updateApprovalTabBadge();
  if (isApprovalTabVisible()) {
    renderApprovalPane();
  }
}, 1000);

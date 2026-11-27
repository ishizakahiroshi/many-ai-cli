// approval-queue-tab.ts
// 承認キュー専用タブ: 全セッションの承認待ちをカード一覧で表示し、その場で回答する。
// スマホの「裁く」用途に最適化。ターミナル表示を経由せずに承認を完結させる。

import { t } from '../i18n.js';
import { activeSessionId, orderSessions, sessions } from './state.js';
import { activateSession } from './session-list.js';
import { extractHubMarkerApproval, isBatchOptions } from './approval-parser.js';
import { isAskUserQuestionPending, pendingApprovalOptions, sendChoice } from './approval.js';
import { approvalRecordFor, hubBlockLines, isApprovalPending, unfoldApproval } from './approval-store.js';
import { apiFetch, showToast } from './util.js';
import { safeClassToken } from './provider-icon.js';
import { approvalHistoryTarget } from './approval-history-target.js';
import { revealChatQuestion } from './chat-history.js';

// 承認タブのバッジカウント更新
function getApprovalTabBadgeEl() {
  return document.getElementById('approval-tab-badge');
}

// 答えを待っている承認があるセッション（Hub の記録の写し。approval-store.ts）。
// AskUserQuestion の告知（端末で答える承認）も含む。
export function pendingSessionIds() {
  return orderSessions()
    .filter(s => s && isApprovalPending(s.id))
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

// プロバイダ色ドット HTML。provider は wrapper の register がそのまま届く値なので、ほかの画面と同じ
// safeClassToken（英小文字・数字・- ・_ だけ）を通してから class 属性へ入れる（innerHTML に入るため）。
// 履歴の行のように className へ直接入れる所も、空白で別のクラスが混ざらないよう同じものを使う。
function providerDotClass(provider) {
  return `aqt-provider-dot prov-shape ${safeClassToken(provider)}`;
}

function providerDotHtml(provider) {
  return `<span class="${providerDotClass(provider)}" aria-hidden="true"></span>`;
}

// セッションのタイトル文字列
export function sessionTitle(s) {
  if (!s) return 'Session';
  const base = s.label || s.last_message || s.first_message
    || (s.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop()
    || `#${s.id}`;
  return `${base}`;
}

export function approvalQuestionContext(options) {
  if (!Array.isArray(options)) return { preamble: '', question: '' };
  const optionProps = options as any;
  const preamble = String(optionProps._preamble || '').trim();
  let question = String(optionProps._question || '').trim();
  if (!question) {
    const meta = options.find((o: any) => o && o._sequentialQuestion);
    question = String((meta as any)?._sequentialQuestion || '').trim();
  }
  if (/^approval needed$/i.test(question)) question = '';
  return { preamble, question };
}

function appendApprovalQuestionContext(card, options) {
  const { preamble, question } = approvalQuestionContext(options);
  const sessionId: number = (card as any)._sessionId;
  const isExpanded = _expandedCards.has(sessionId);

  const addClampedBlock = (text: string, extraClass: string) => {
    if (!text) return;
    const wrap = document.createElement('div');
    wrap.className = `aqt-question ${extraClass}${isExpanded ? '' : ' is-clamped'}`;
    wrap.textContent = text;
    card.appendChild(wrap);

    // トグルボタン: 初期は hidden で挿入し、描画後 scrollHeight > clientHeight + 1 のとき可視化する
    const toggleBtn = document.createElement('button');
    toggleBtn.type = 'button';
    toggleBtn.className = 'ui-button ui-button--secondary ui-button--regular aqt-question-toggle';
    toggleBtn.textContent = isExpanded ? t('approval_tab_collapse') : t('approval_tab_expand');
    toggleBtn.hidden = true;
    card.appendChild(toggleBtn);

    toggleBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      const expanded = _expandedCards.has(sessionId);
      if (expanded) {
        _expandedCards.delete(sessionId);
        card.classList.remove('aqt-card--expanded');
      } else {
        _expandedCards.add(sessionId);
        card.classList.add('aqt-card--expanded');
      }
      const nowExpanded = _expandedCards.has(sessionId);
      wrap.classList.toggle('is-clamped', !nowExpanded);
      toggleBtn.textContent = nowExpanded ? t('approval_tab_collapse') : t('approval_tab_expand');
      _lastPaneSig = null;
    });

    // 描画後にオーバーフロー判定: clamp 状態でテキストが収まらない場合のみトグルを出す
    requestAnimationFrame(() => {
      if (isExpanded) {
        toggleBtn.hidden = false;
        return;
      }
      toggleBtn.hidden = !(wrap.scrollHeight > wrap.clientHeight + 1);
    });
  };

  addClampedBlock(preamble, 'aqt-question--preamble');
  addClampedBlock(question, 'aqt-question--batch');

  // expanded 状態をカード DOM に反映
  if (isExpanded) card.classList.add('aqt-card--expanded');
}

// 単一承認の選択肢ボタン群を生成（options はフラット配列前提）
export function renderOptionButtons(container, sessionId, options) {
  container.innerHTML = '';
  if (!Array.isArray(options) || options.length === 0) return;

  options.forEach((opt) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'ui-button ui-button--secondary ui-button--regular aqt-option-btn';
    if (opt.isCurrent) btn.classList.add('is-current');

    const label = opt.label || opt.send_text || String(opt.num);
    // 番号とラベルを分けて span 構造で持たせる（PC は CSS 側 ::after で ". " を補い従来表示、
    // スマホは mobile-home.css 側で丸バッジに override する／Q1 番号併記の視覚スタイル）
    const numEl = document.createElement('span');
    numEl.className = 'aqt-option-num';
    numEl.textContent = String(opt.num);
    const labelEl = document.createElement('span');
    labelEl.className = 'aqt-option-label';
    labelEl.textContent = label;
    btn.appendChild(numEl);
    btn.appendChild(labelEl);
    btn.dataset.sendText = opt.send_text || '';
    btn.dataset.optNum = String(opt.num);

    btn.addEventListener('click', () => {
      // 既存の sendChoice 経路を流用（回答済み記録 / hideActionBar / chatHistory 連動を含む）
      sendChoice(sessionId, opt.num);
      // 即時フィードバックとしてボタンを無効化
      container.querySelectorAll('.aqt-option-btn').forEach(b => {
        (b as HTMLButtonElement).disabled = true;
      });
      btn.classList.add('chosen');
      // Hub 側が resolved を書き終える猶予を置いてから履歴を取り直す
      setTimeout(() => { void loadApprovalHistory(true); }, 800);
    });

    container.appendChild(btn);
  });
}

// 直前に描画した内容の署名。毎秒の定期再描画で内容が変わっていないのに
// pane.innerHTML を破棄・再生成すると、クリックの mousedown と mouseup の間に
// 再描画が挟まりボタンが入れ替わって click イベントが発火しなくなる（押しても遷移しない）。
// 署名が一致する間は DOM をそのまま据え置く。
let _lastPaneSig: string | null = null;

// 展開中のカード ID セット（expand toggle の状態をリレンダリング間で保持）
let _expandedCards: Set<number> = new Set();

// 描画に影響する状態だけを集めた署名を作る
function approvalPaneSig(ids: number[]): string {
  // 開いているセッションの一覧も入れる。履歴の行を押せるかどうかはそれで変わる。
  return `${_historyVersion}\u0000${Array.from(sessions.keys()).join(',')}\u0000` + JSON.stringify(ids.map((id) => {
    const s = sessions.get(id);
    const options = pendingApprovalOptions(id);
    return {
      id,
      title: sessionTitle(s),
      provider: s ? s.provider : '',
      active: id === activeSessionId,
      multiQ: isAskUserQuestionPending(id),
      batch: Array.isArray(options) && isBatchOptions(options),
      ctx: approvalQuestionContext(options),
      opts: Array.isArray(options)
        ? options.map((o) => `${o.num}\u0000${o.label || ''}\u0000${o.send_text || ''}\u0000${o.isCurrent ? 1 : 0}`)
        : null,
      expanded: _expandedCards.has(id),
    };
  }));
}

function batchSignature(sessionId: number, options: any[]): string | null {
  const summary = (options as any)?._summary;
  const session = sessions.get(sessionId);
  if (!summary || !session) return null;
  return `${String(session.provider || '').trim().toLowerCase()}\0${String(summary.command || '').trim()}\0${String(summary.risk || 'mid')}\0${String(session.cwd || '').trim()}`;
}

async function runBatchAction(body: any): Promise<void> {
  const res = await apiFetch('/api/approval/batch', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error('batch action failed');
  window.dispatchEvent(new CustomEvent('approval-queue-updated'));
}

// 承認パネル全体を描画
// --- 承認履歴（台帳 /api/approval-history）---
//
// 保留中の承認はブラウザのメモリにしか無く、タブが取りこぼすと誰も復元できない。
// 台帳は Hub 側に必ず残るので、ここは「直近と過去を必ず見られる」唯一の窓になる。
// 1 秒ごとの定期再描画では取りに行かない（描画は _historyVersion 経由で追従する）。

let _historyRows: any[] = [];
let _historyVersion = 0;
let _historyFetchedAt = 0;
let _historyInFlight = false;
const HISTORY_MIN_INTERVAL_MS = 2000;
const HISTORY_LIMIT = 50;

export async function loadApprovalHistory(force = false): Promise<void> {
  const now = Date.now();
  if (_historyInFlight) return;
  if (!force && now - _historyFetchedAt < HISTORY_MIN_INTERVAL_MS) return;
  _historyInFlight = true;
  try {
    const res = await apiFetch(`/api/approval-history?limit=${HISTORY_LIMIT}`);
    if (!res.ok) return;
    const body = await res.json();
    const rows = Array.isArray(body?.approvals) ? body.approvals : [];
    _historyRows = rows;
    _historyVersion++;
    _historyFetchedAt = Date.now();
    renderApprovalPane();
  } catch (_) {
    // 取得できないときは前回の内容を残す（空にすると「履歴が消えた」ように見えるため）
  } finally {
    _historyInFlight = false;
  }
}

// 台帳の 1 行から質問文を作る。マーカー由来の行は question が空で block に原文が入るので、
// 承認パネルと同じパーサで解く（解釈器を 2 本にしない）。
function approvalRowSummary(row: any): string {
  const direct = String(row?.question || '').trim();
  if (direct) return direct;
  const block = String(row?.block || '');
  if (!block) return '';
  try {
    const options = extractHubMarkerApproval(hubBlockLines(block));
    const { preamble, question } = approvalQuestionContext(options);
    if (question) return question;
    if (preamble) return preamble;
  } catch (_) { /* 壊れたブロックは下の素朴な抽出へ落とす */ }
  const plain = block.split(/\r\n|\r|\n/)
    .map(l => l.trim())
    .find(l => l && !l.startsWith('[') && !/^\d+\.\s/.test(l) && !/^N\.\s/.test(l));
  return plain || '';
}

function approvalRowTime(row: any): string {
  const raw = String(row?.detected_at || '');
  if (!raw) return '';
  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) return '';
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

function approvalRowSessionLabel(row: any): string {
  const live = Number(row?.session_id) || 0;
  const dir = String(row?.cwd || '').replace(/\\/g, '/').split('/').filter(Boolean).pop() || '';
  if (live && dir) return `#${live} ${dir}`;
  if (live) return `#${live}`;
  return dir;
}

// セッションを開いて、指定のタブを表示する。
function openSessionTab(id: number, tab: 'terminal' | 'chat') {
  activateSession(id);
  (document.querySelector(`#unified-tab-bar .view-tab[data-tab="${tab}"]`) as HTMLButtonElement | null)?.click();
}

function historyRowTarget(row: any) {
  const id = Number(row?.session_id) || 0;
  return approvalHistoryTarget(row, sessions.get(id), approvalRecordFor(id));
}

// 履歴の行を押したとき。行き先は押した時点の状態で決め直す（描いた後に回答・終了することがある）。
async function openApprovalHistoryRow(row: any): Promise<void> {
  const id = Number(row?.session_id) || 0;
  const target = historyRowTarget(row);
  if (target === 'gone') {
    showToast(t('approval_history_session_ended'));
    _lastPaneSig = null;
    renderApprovalPane();
    return;
  }
  if (target === 'current') {
    unfoldApproval(id); // 畳んだままだと、移動しても質問が見えない
    openSessionTab(id, 'terminal');
    return;
  }
  openSessionTab(id, 'chat');
  const found = await revealChatQuestion(id, approvalRowSummary(row), String(row?.detected_at || ''));
  if (!found) showToast(t('approval_history_question_not_found'));
}

function appendApprovalHistorySection(pane: HTMLElement): void {
  const wrap = document.createElement('section');
  wrap.className = 'aqt-history';

  const head = document.createElement('div');
  head.className = 'aqt-history-head';
  head.textContent = t('approval_history_title');
  wrap.appendChild(head);

  if (_historyRows.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'aqt-history-empty';
    empty.textContent = t('approval_history_empty');
    wrap.appendChild(empty);
    pane.appendChild(wrap);
    return;
  }

  const list = document.createElement('ul');
  list.className = 'aqt-history-list';
  for (const row of _historyRows) {
    const li = document.createElement('li');
    const pending = String(row?.state || '') !== 'resolved';
    li.className = `aqt-history-row ${pending ? 'is-pending' : 'is-resolved'}`;

    const time = document.createElement('span');
    time.className = 'aqt-history-time';
    time.textContent = approvalRowTime(row);
    li.appendChild(time);

    const dot = document.createElement('span');
    dot.className = providerDotClass(row?.provider);
    dot.setAttribute('aria-hidden', 'true');
    li.appendChild(dot);

    const who = document.createElement('span');
    who.className = 'aqt-history-session';
    who.textContent = approvalRowSessionLabel(row);
    li.appendChild(who);

    const q = document.createElement('span');
    q.className = 'aqt-history-q';
    q.textContent = approvalRowSummary(row) || t('approval_history_no_question');
    li.appendChild(q);

    const answer = document.createElement('span');
    answer.className = 'aqt-history-answer';
    const target = historyRowTarget(row);
    answer.textContent = pending
      ? t(target === 'gone' ? 'approval_history_pending_ended' : 'approval_history_pending')
      : String(row?.selected_text || '').trim();
    li.appendChild(answer);

    if (target !== 'gone') {
      li.classList.add('is-link');
      li.setAttribute('role', 'button');
      li.tabIndex = 0;
      li.title = t('approval_history_open');
      li.addEventListener('click', () => { void openApprovalHistoryRow(row); });
      li.addEventListener('keydown', (e) => {
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        void openApprovalHistoryRow(row);
      });
    }

    list.appendChild(li);
  }
  wrap.appendChild(list);
  pane.appendChild(wrap);
}

// 承認待ちのカード一覧を描き、その下へ履歴を足す。
// 履歴の内容は署名（approvalPaneSig）に _historyVersion として入っているので、
// 据え置き判定を通ったときは履歴も描き直さない。
function renderApprovalPane() {
  const before = _lastPaneSig;
  renderApprovalPendingPane();
  const pane = document.getElementById('approval-pane');
  if (!pane) return;
  if (_lastPaneSig !== before || !pane.querySelector('.aqt-history')) {
    appendApprovalHistorySection(pane);
  }
}

function renderApprovalPendingPane() {
  const pane = document.getElementById('approval-pane');
  if (!pane) return;

  const ids = pendingSessionIds();

  const signatureGroups = new Map<string, number[]>();
  for (const id of ids) {
    const options = pendingApprovalOptions(id) as any[];
    if (!Array.isArray(options)) continue;
    const signature = batchSignature(id, options);
    if (signature) signatureGroups.set(signature, [...(signatureGroups.get(signature) || []), id]);
  }

  // 内容が前回と同一なら DOM を据え置く（毎秒再生成による死にクリック防止）。
  // pane が空（初回 / 外部クリア）の場合は署名一致でも描画する。
  const sig = approvalPaneSig(ids);
  if (sig === _lastPaneSig && pane.childElementCount > 0) {
    updateApprovalTabBadge();
    return;
  }
  _lastPaneSig = sig;

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
    const options = pendingApprovalOptions(id);
    const isMultiQ = isAskUserQuestionPending(id);
    const isActive = id === activeSessionId;

    const card = document.createElement('div');
    card.className = 'aqt-card' + (isActive ? ' is-active-session' : '');
    card.dataset.sessionId = String(id);
    (card as any)._sessionId = id;

    const signature = Array.isArray(options) ? batchSignature(id, options as any[]) : null;
    const group = signature ? signatureGroups.get(signature) : null;
    const summary = (options as any)?._summary;
    if (signature && group && group.length > 1 && group[0] === id) {
      const controls = document.createElement('div');
      controls.className = 'aqt-batch-controls';
      const title = document.createElement('span');
      title.textContent = `同一承認 ${group.length} 件`;
      controls.appendChild(title);
      if (summary?.risk === 'low') {
        const approve = document.createElement('button');
        approve.type = 'button'; approve.className = 'ui-button ui-button--primary ui-button--regular'; approve.textContent = 'まとめて許可';
        approve.addEventListener('click', async () => {
          try { await runBatchAction({ signature, action: 'approve' }); showToast(`${group.length} 件を許可しました`); }
          catch { showToast('一括許可に失敗しました'); }
        });
        controls.appendChild(approve);
        const auto = document.createElement('button');
        auto.type = 'button'; auto.className = 'ui-button ui-button--secondary ui-button--regular'; auto.textContent = '自動承認に追加';
        auto.addEventListener('click', async () => {
          try { await runBatchAction({ signature, action: 'auto_rule' }); showToast('自動承認ルールを追加しました'); }
          catch { showToast('自動承認ルールを追加できませんでした'); }
        });
        controls.appendChild(auto);
      }
      card.appendChild(controls);
    }

    // ── カードヘッダー ──
    const header = document.createElement('div');
    header.className = 'aqt-card-header';

    const dotHtml = s ? providerDotHtml(s.provider) : '';
    const titleEl = document.createElement('div');
    titleEl.className = 'aqt-card-title';
    titleEl.innerHTML = `${dotHtml}<span class="aqt-session-id">#${id}</span> <span class="aqt-session-name">${escapeHtml(sessionTitle(s))}</span>`;

    const goBtn = document.createElement('button');
    goBtn.type = 'button';
    goBtn.className = 'ui-button ui-button--secondary ui-button--regular aqt-go-terminal-btn';
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

    const denyRest = document.createElement('button');
    denyRest.type = 'button'; denyRest.className = 'ui-button ui-button--danger ui-button--regular aqt-deny-rest-btn'; denyRest.textContent = 'このセッションの残りを拒否';
    denyRest.addEventListener('click', async () => {
      try { await runBatchAction({ action: 'deny_session', session_id: id }); showToast('このセッションの承認を拒否しました'); }
      catch { showToast('一括拒否に失敗しました'); }
    });
    card.appendChild(denyRest);

    // ── 質問文 ──
    if (isMultiQ && (!options || options.length === 0)) {
      const qEl = document.createElement('div');
      qEl.className = 'aqt-question aqt-question--multiq';
      qEl.textContent = t('approval_tab_multiq_hint');
      card.appendChild(qEl);

      const goBtn2 = document.createElement('button');
      goBtn2.type = 'button';
      goBtn2.className = 'ui-button ui-button--secondary ui-button--regular aqt-option-btn';
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
        goBtn2.className = 'ui-button ui-button--secondary ui-button--regular aqt-option-btn';
        goBtn2.textContent = t('approval_tab_go_terminal_full');
        goBtn2.addEventListener('click', () => {
          activateSession(id);
          const termTab = document.querySelector('#unified-tab-bar .view-tab[data-tab="terminal"]') as HTMLButtonElement | null;
          termTab?.click();
        });
        card.appendChild(goBtn2);
      } else {
        // 通常承認: 選択肢ボタンを直接表示
        appendApprovalQuestionContext(card, options);

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
      // 告知（AskUserQuestion）と画面のパーサが読めない記録。画面は端末の文字から承認を作らないので
      // 「検出中」ではなく、端末で答える承認として出す。
      qEl.className = 'aqt-question aqt-question--detecting';
      qEl.textContent = t('approval_tab_answer_in_terminal');
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
// 拒否 Undo の猶予時間（ms）。トースト表示時間と確定タイマーの単一ソース。
const UNDO_GRACE_MS = 3000;

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
  // Undo 猶予（UNDO_GRACE_MS）と同じ時間トーストを表示し、視覚と挙動を一致させる。
  showToast(msg, undefined, UNDO_GRACE_MS);

  // トーストに Undo ボタンを追加。確定・Undo いずれの経路でも確実に除去できるよう
  // 関数スコープに参照を持たせ、共通の後処理 cleanupUndoBtn() で remove する。
  let undoBtn: HTMLButtonElement | null = null;
  const cleanupUndoBtn = () => {
    if (undoBtn) {
      undoBtn.remove();
      undoBtn = null;
    }
  };

  const toastEl = document.getElementById('toast');
  if (toastEl) {
    undoBtn = document.createElement('button');
    undoBtn.className = 'ui-button ui-button--secondary ui-button--regular aqt-undo-toast-btn';
    undoBtn.textContent = undoLabel;
    undoBtn.addEventListener('click', () => {
      if (undone) return;
      undone = true;
      if (_undoTimer) { clearTimeout(_undoTimer); _undoTimer = null; }
      cleanupUndoBtn();
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
    // 確定パスでも孤立した Undo ボタンを #toast から確実に除去する。
    cleanupUndoBtn();
    if (!undone) {
      sendChoice(sessionId, opt.num);
    }
  }, UNDO_GRACE_MS);
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
    void loadApprovalHistory(true);
  }
});

// 承認キャッシュが変わるたびに呼ばれる既存フック
// approval-queue.ts の updateApprovalQueue と同様に window イベントで連動
window.addEventListener('approval-queue-updated', () => {
  updateApprovalTabBadge();
  if (isApprovalTabVisible()) {
    renderApprovalPane();
    void loadApprovalHistory();
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

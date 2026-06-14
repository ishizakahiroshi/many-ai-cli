// workflow-modal.ts — Workflow ライブ進捗の「トリガーピル + 自前描画モーダル」。
//
// 役割（plan_workflow-progress-modal.md C2 + 完了振り返り拡張）:
//   - アクティブセッションの xterm バッファを短間隔ポーリングし、workflow-progress.ts で構造化。
//   - Workflow を検出している間「⚙ Workflow（N 走行中）」ピルを入力欄上部に出す。
//   - 走行が終わっても **完了スナップショットをメモリ保持し、ピル/モーダルを残す**ので、
//     「終わったやつの中身を後からチェックする」用途に使える。ユーザーが ✕ で閉じるか、
//     次の別 Workflow が始まるまで残る（端末バッファから流れても保持分で描画できる）。
//   - 保持はセッションが生きている間だけ（ディスク非永続＝plan の方針を維持）。
//     session_removed で removeWorkflowSnapshot によりクリアする。
//   - ピルのクリックで中央モーダル。生 VT ミラーではなく構造化モデルから自前 DOM 描画。
//   - 解釈できないときはピルを出さない / モーダルに「解釈できませんでした」を出す。
//
// 純パース部は workflow-progress.ts（DOM/i18n 非依存）に分離。ここは描画と配線だけ。

import { t } from '../i18n.js';
import { activeSessionId } from './state.js';
import { scanBuffer } from './terminal.js';
import { parseWorkflowProgress, WorkflowProgress, WfAgentState } from './workflow-progress.js';

// 進捗ブロックはビューポート近傍に出るので、末尾の十分な行数だけ見れば足りる。
const SCAN_LINES = 200;
const POLL_MS = 800;
// 走行中スナップショットが連続で未検出になったら「進捗フレームが止まった＝実質完了」とみなし
// 完了表示へ倒す（端末から流れた / 終わった、を VT では区別できないため idle 判定と同じ思想）。
const SETTLE_MISS_LIMIT = 5;

// セッションごとの最新 Workflow スナップショット（セッション生存中のみ・ディスク非永続）。
interface WfSnapshot {
  result: WorkflowProgress;
  /** 走行終了（完了/最終状態で固定）したか。true で done 表示。 */
  settled: boolean;
  /** ユーザーが ✕ で閉じたか。true の間は同一 Workflow を再表示しない。 */
  dismissed: boolean;
  /** 同一 Workflow 判定用シグネチャ（name + フェーズ/エージェント構成）。 */
  sig: string;
}

const snapshots = new Map<number, WfSnapshot>();
const missCounts = new Map<number, number>();
let modalOpen = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

function sigOf(r: WorkflowProgress): string {
  return r.name + '||' + r.phases
    .map(p => p.title + ':' + p.agents.map(a => a.label).slice().sort().join(','))
    .join('|');
}

// ── DOM 参照 ──────────────────────────────────────────────────────────────────

function getPill(): HTMLElement | null {
  return document.getElementById('workflow-progress-pill');
}

function ensurePill(): HTMLElement {
  let pill = getPill();
  if (pill) return pill;
  pill = document.createElement('div');
  pill.id = 'workflow-progress-pill';
  pill.hidden = true;
  pill.innerHTML =
    '<button type="button" class="wf-pill-open" aria-haspopup="dialog">' +
    '<span class="wf-pill-spinner live-spinner" aria-hidden="true"></span>' +
    '<span class="wf-pill-text"></span></button>' +
    '<button type="button" class="wf-pill-dismiss" aria-label="dismiss">✕</button>';
  pill.querySelector('.wf-pill-open')?.addEventListener('click', toggleWorkflowModal);
  pill.querySelector('.wf-pill-dismiss')?.addEventListener('click', dismissActive);
  // 入力欄直上のライブ進捗ピル（#terminal-live-status）の直前に置くと導線が一貫する。
  const liveStatus = document.getElementById('terminal-live-status');
  if (liveStatus && liveStatus.parentElement) {
    liveStatus.parentElement.insertBefore(pill, liveStatus);
  } else {
    const outer = document.getElementById('input-bar-outer');
    if (outer) outer.insertBefore(pill, outer.firstChild);
    else document.body.appendChild(pill);
  }
  return pill;
}

// ── ポーリング ────────────────────────────────────────────────────────────────

function poll(): void {
  const sid = activeSessionId;
  if (sid !== null) {
    const result = parseWorkflowProgress(scanBuffer(sid, SCAN_LINES));
    if (result.detected) {
      missCounts.set(sid, 0);
      const sig = sigOf(result);
      const prev = snapshots.get(sid);
      // 別 Workflow（sig 変化）になったら dismiss を解除して再表示する。
      const keepDismissed = !!(prev && prev.dismissed && prev.sig === sig);
      snapshots.set(sid, { result, settled: !result.running, dismissed: keepDismissed, sig });
    } else {
      // 未検出。完了スナップショットは振り返り用に保持し続ける（消さない）。
      // 走行中のまま進捗が途切れた場合は SETTLE_MISS_LIMIT 連続で完了表示へ倒す。
      const prev = snapshots.get(sid);
      if (prev && !prev.settled) {
        const miss = (missCounts.get(sid) || 0) + 1;
        missCounts.set(sid, miss);
        if (miss >= SETTLE_MISS_LIMIT) prev.settled = true;
      }
    }
  }
  renderPill();
  if (modalOpen) renderModalBody();
}

function activeSnapshot(): WfSnapshot | null {
  const sid = activeSessionId;
  if (sid === null) return null;
  return snapshots.get(sid) || null;
}

function renderPill(): void {
  const pill = ensurePill();
  const snap = activeSnapshot();
  if (!snap || snap.dismissed || !snap.result.detected || snap.result.totalCount === 0) {
    pill.hidden = true;
    if (modalOpen) closeWorkflowModal();
    return;
  }
  pill.hidden = false;
  const done = snap.settled || !snap.result.running;
  pill.classList.toggle('wf-done', done);
  const textEl = pill.querySelector('.wf-pill-text') as HTMLElement | null;
  if (textEl) {
    textEl.textContent = done
      ? t('wf_progress_pill_done', { n: snap.result.totalCount })
      : t('wf_progress_pill_running', { n: snap.result.runningCount });
  }
  const openBtn = pill.querySelector('.wf-pill-open') as HTMLElement | null;
  if (openBtn) openBtn.title = done ? t('wf_progress_done') : t('wf_progress_running');
}

function dismissActive(ev?: Event): void {
  ev?.stopPropagation();
  const snap = activeSnapshot();
  if (snap) snap.dismissed = true;
  if (modalOpen) closeWorkflowModal();
  renderPill();
}

// ── モーダル ──────────────────────────────────────────────────────────────────

let keyHandler: ((e: KeyboardEvent) => void) | null = null;
let downHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;

function toggleWorkflowModal(): void {
  if (modalOpen) { closeWorkflowModal(); return; }
  openWorkflowModal();
}

export function openWorkflowModal(): void {
  if (document.getElementById('workflow-modal')) { modalOpen = true; return; }

  const overlay = document.createElement('div');
  overlay.id = 'workflow-modal';

  const box = document.createElement('div');
  box.className = 'wf-modal-box';
  box.setAttribute('role', 'dialog');
  box.setAttribute('aria-modal', 'true');

  const header = document.createElement('div');
  header.className = 'wf-modal-header';
  const title = document.createElement('span');
  title.className = 'wf-modal-title';
  title.textContent = t('wf_progress_title');
  header.appendChild(title);
  const counts = document.createElement('span');
  counts.className = 'wf-modal-counts';
  header.appendChild(counts);
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'wf-modal-close';
  closeBtn.textContent = '✕';
  closeBtn.setAttribute('aria-label', t('settings_close'));
  closeBtn.addEventListener('click', closeWorkflowModal);
  header.appendChild(closeBtn);
  box.appendChild(header);

  const body = document.createElement('div');
  body.className = 'wf-modal-body';
  box.appendChild(body);

  overlay.appendChild(box);
  document.body.appendChild(overlay);
  modalOpen = true;
  renderModalBody();

  // 外側クリック / Esc / ピル再クリックで閉じる（expand-popup.ts 踏襲）。
  downHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    if (box.contains(target)) return;
    if ((target as HTMLElement).closest?.('#workflow-progress-pill')) return; // 再クリックは toggle に委ねる
    closeWorkflowModal();
  };
  keyHandler = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    closeWorkflowModal();
  };
  setTimeout(() => {
    if (downHandler) {
      document.addEventListener('mousedown', downHandler, true);
      document.addEventListener('touchstart', downHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', keyHandler, true);
}

export function closeWorkflowModal(): void {
  modalOpen = false;
  const existing = document.getElementById('workflow-modal');
  if (existing) existing.remove();
  if (keyHandler) { document.removeEventListener('keydown', keyHandler, true); keyHandler = null; }
  if (downHandler) {
    document.removeEventListener('mousedown', downHandler, true);
    document.removeEventListener('touchstart', downHandler, true);
    downHandler = null;
  }
}

function stateIcon(state: WfAgentState): HTMLElement {
  const span = document.createElement('span');
  span.className = 'wf-agent-icon wf-state-' + state;
  if (state === 'running') {
    span.classList.add('live-spinner');
    span.setAttribute('aria-hidden', 'true');
  } else {
    span.textContent = state === 'done' ? '✓' : state === 'failed' ? '✗' : '○';
  }
  return span;
}

function renderModalBody(): void {
  const overlay = document.getElementById('workflow-modal');
  if (!overlay) return;
  const counts = overlay.querySelector('.wf-modal-counts') as HTMLElement | null;
  const body = overlay.querySelector('.wf-modal-body') as HTMLElement | null;
  if (!body) return;

  const snap = activeSnapshot();
  const r = snap && !snap.dismissed ? snap.result : null;

  // 解釈不能（検出が外れた / フォーマット不一致）: クラッシュさせず案内を出す。
  if (!r || !r.detected || r.totalCount === 0) {
    if (counts) counts.textContent = '';
    body.innerHTML = '';
    const empty = document.createElement('div');
    empty.className = 'wf-modal-empty';
    empty.textContent = t('wf_progress_unparsed');
    body.appendChild(empty);
    return;
  }

  const done = snap!.settled || !r.running;
  if (counts) {
    const parts = [
      t('wf_progress_counts_running', { n: r.runningCount }),
      t('wf_progress_counts_done', { n: r.doneCount }),
    ];
    if (r.failedCount > 0) parts.push(t('wf_progress_counts_failed', { n: r.failedCount }));
    counts.textContent = parts.join(' · ');
  }

  body.innerHTML = '';

  // 進捗バー（percent があれば）。
  if (r.percent !== null) {
    const barWrap = document.createElement('div');
    barWrap.className = 'wf-progress-bar';
    const fill = document.createElement('div');
    fill.className = 'wf-progress-fill';
    fill.style.width = Math.max(0, Math.min(100, r.percent)) + '%';
    if (done) fill.classList.add('wf-progress-fill-done');
    barWrap.appendChild(fill);
    const pct = document.createElement('span');
    pct.className = 'wf-progress-pct';
    pct.textContent = r.percent + '%';
    barWrap.appendChild(pct);
    body.appendChild(barWrap);
  }

  if (r.name) {
    const nameEl = document.createElement('div');
    nameEl.className = 'wf-modal-name';
    nameEl.textContent = r.name;
    body.appendChild(nameEl);
  }

  for (const phase of r.phases) {
    const phaseEl = document.createElement('div');
    phaseEl.className = 'wf-phase';
    if (phase.title) {
      const ph = document.createElement('div');
      ph.className = 'wf-phase-title';
      ph.textContent = phase.title;
      phaseEl.appendChild(ph);
    }
    const list = document.createElement('div');
    list.className = 'wf-agent-list';
    for (const agent of phase.agents) {
      const row = document.createElement('div');
      row.className = 'wf-agent-row wf-agent-' + agent.state;
      row.appendChild(stateIcon(agent.state));
      const lbl = document.createElement('span');
      lbl.className = 'wf-agent-label';
      lbl.textContent = agent.label;
      row.appendChild(lbl);
      list.appendChild(row);
    }
    phaseEl.appendChild(list);
    body.appendChild(phaseEl);
  }
}

// ── 外部 API ─────────────────────────────────────────────────────────────────

/** session_removed 時に保持スナップショットをクリアする（メモリ保持はセッション生存中のみ）。 */
export function removeWorkflowSnapshot(sessionId: number): void {
  snapshots.delete(sessionId);
  missCounts.delete(sessionId);
  if (sessionId === activeSessionId) renderPill();
}

/** DOM 準備後に呼ぶ。ポーリングを開始し、Workflow 検出時のみピルを出す。 */
export function initWorkflowProgress(): void {
  ensurePill();
  if (pollTimer) return;
  pollTimer = setInterval(poll, POLL_MS);
}

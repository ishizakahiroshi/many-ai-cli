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
import type { WfAgentDetail, WorkflowProgress as HubWorkflowProgress } from '../types/proto.js';
import { activeSessionId, sessions } from './state.js';
import { scanBuffer } from './terminal.js';
// 状態記号はサイドバーのカード・セッション帯（タブ）・ライブ帯と同じ SVG を借りる。
// ワークフローだけ別の丸（旧 .live-spinner の CSS スピナー）を出すと、同じ「実行中」が
// 画面内で 2 通りの形・2 通りの色になる。
import { stateIconSvgHtml } from './session-list.js';
import { parseWorkflowProgress, WorkflowProgress, WfAgentState } from './workflow-progress.js';
import {
  extrapolatedWorkflowElapsedSec,
  getHubWorkflowEntry,
  getSubagentTreeEntry,
  getWorkflowLedger,
  isHubWorkflowAuthoritative,
  recordWorkflowDoneLabels,
  removeWorkflowStore,
  setHubWorkflowProgress,
} from './workflow-store.js';
// 表示行への組み立ては純関数側（DOM/i18n 非依存）に分離済み。ここは描画と配線だけ
// （子 plan: plan_subagent-tree-popup_c5_web-popup.md の C1/C2 と同じ分け方）。
import { buildSubagentTreeRows, SubagentRowState, SubagentTreeRow, SubagentTreeSummary, SubagentTreeView } from './subagent-tree.js';

// 進捗ブロックはビューポート近傍に出るので、末尾の十分な行数だけ見れば足りる。
const SCAN_LINES = 200;
const POLL_MS = 800;
// 走行中スナップショットが連続で未検出になったら「進捗フレームが止まった＝実質完了」とみなし
// 完了表示へ倒す（端末から流れた / 終わった、を VT では区別できないため idle 判定と同じ思想）。
const SETTLE_MISS_LIMIT = 5;
// 直近が live（背景 Workflow の N/M agents done）だった場合、全信号（要約も Waiting 行も）
// が窓外へ流れても即 settle せず、長い backstop 猶予（≈32 秒 = 40 ポール）まで待つ。
// 通常のスクロール欠落（数秒）では発火しない。非 live は SETTLE_MISS_LIMIT のまま。
const LIVE_SETTLE_MISS_LIMIT = 40;

// セッションごとの最新 Workflow スナップショット（セッション生存中のみ・ディスク非永続）。
interface WfSnapshot {
  result: WorkflowProgress;
  /** 走行終了（完了/最終状態で固定）したか。true で done 表示。 */
  settled: boolean;
  /** ユーザーが ✕ で閉じたか。true の間は同一 Workflow を再表示しない。 */
  dismissed: boolean;
  /** 同一 Workflow 判定用シグネチャ（name + フェーズ/エージェント構成）。 */
  sig: string;
  /** Hub WebSocket またはローカル VT fallback。 */
  source: 'hub' | 'local';
}

const snapshots = new Map<number, WfSnapshot>();
const missCounts = new Map<number, number>();
// サブエージェントの木のチップ/モーダルは WfSnapshot を持たない（純関数側に状態が
// 無いため）。✕ で閉じた事実だけをセッション単位で覚える。木が空になった
// （getSubagentTreeEntry が null を返す）ときに削除し、次に現れる木は必ず
// 未読状態から始める（親 plan 方針4の「今のまとまり」区切りと同じ考え方）。
const subagentDismissed = new Set<number>();
// フレーム凍結検出: 走行中なのに frameSig が連続で不変＝スピナーが止まっている
// （＝完了したのに走行中グリフがバッファに残っている）状態。連続一致回数を数える。
const freezeCounts = new Map<number, number>();
const lastFrameSig = new Map<number, string>();
let modalOpen = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

function sigOf(r: WorkflowProgress): string {
  return r.name + '||' + r.phases
    .map(p => p.title + ':' + p.agents.map(a => a.label).slice().sort().join(','))
    .join('|');
}

// tasks output（C1 実測）の state は 'done' / 'error' のみ確認済み（'pending' /
// 'running' は想定値として許容）。'error' 専用のアイコン・CSS バケットは無いため、
// VT/journal 側が既に持つ 'failed'（✗ 表示・赤色）へ統合する。
function hubAgentState(state: string): WfAgentState {
  if (state === 'error') return 'failed';
  return state === 'running' || state === 'done' || state === 'failed' || state === 'pending'
    ? state
    : 'pending';
}

function workflowFromHub(progress: HubWorkflowProgress, receivedAt: number, now: number): WorkflowProgress {
  const phases = (progress.phases || []).map(phase => ({
    title: String(phase.title || ''),
    agents: (phase.agents || []).map(agent => ({
      label: String(agent.label || ''),
      state: hubAgentState(String(agent.state || '')),
      glyph: '',
      metrics: agent.metrics,
      detail: agent.detail,
    })),
  }));
  const total = Math.max(0, Number(progress.total || 0));
  const done = Math.max(0, Number(progress.done || 0));
  const waiting = Math.max(0, Number(progress.waiting_dynamic || 0));
  const settled = !!progress.settled;
  const running = !settled && (
    Number(progress.running || 0) > 0 ||
    Number(progress.pending || 0) > 0 ||
    waiting > 0 ||
    (total > 0 && done < total)
  );
  const entry = { progress, receivedAt };
  const hasElapsed = typeof progress.elapsed_sec === 'number' && progress.elapsed_sec >= 0;
  return {
    detected: !!progress.detected,
    running,
    live: !settled && total > 0 && done < total,
    waitingDynamic: waiting,
    name: String(progress.name || ''),
    phases,
    runningCount: settled ? 0 : Math.max(0, Number(progress.running || 0)),
    doneCount: done,
    failedCount: Math.max(0, Number(progress.failed || 0)),
    totalCount: total,
    percent: Number.isFinite(progress.percent) ? Math.max(0, Math.min(100, progress.percent)) : null,
    frameSig: 'hub:' + [progress.source, progress.name, done, total, progress.running,
      progress.pending, waiting, progress.settled, progress.settled_by].join('|'),
    elapsedSec: hasElapsed ? extrapolatedWorkflowElapsedSec(entry, now) : undefined,
    tokensRaw: String(progress.tokens_raw || ''),
    pendingCount: Math.max(0, Number(progress.pending || 0)),
    settledBy: String(progress.settled_by || ''),
    authority: 'hub',
    taskDetailSource: String(progress.task_detail_source || ''),
  };
}

function applyHubSnapshot(sessionId: number, now = Date.now()): boolean {
  const entry = getHubWorkflowEntry(sessionId);
  if (!entry || !isHubWorkflowAuthoritative(entry, now)) return false;
  const result = workflowFromHub(entry.progress, entry.receivedAt, now);
  const sig = sigOf(result);
  const prev = snapshots.get(sessionId);
  // sig は Hub パーサとローカルパーサで同一 workflow でも書式が揃わないことが
  // あるため、ソース切替をまたぐときは dismiss を 1 回引き継ぐ（✕ で閉じた
  // ピルが fallback 切替の瞬間に復活する問題 — 敵対レビュー 2026-08-05 F3）。
  const keepDismissed = !!(prev && prev.dismissed && (prev.sig === sig || prev.source !== 'hub'));
  snapshots.set(sessionId, {
    result,
    settled: !!entry.progress.settled,
    dismissed: keepDismissed,
    sig,
    source: 'hub',
  });
  missCounts.set(sessionId, 0);
  freezeCounts.set(sessionId, 0);
  return true;
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
    '<span class="wf-pill-spinner wf-running" aria-hidden="true">' + stateIconSvgHtml('ring') + '</span>' +
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
    const now = Date.now();
    const hubEntry = getHubWorkflowEntry(sid);
    if (!applyHubSnapshot(sid, now)) {
      const result = parseWorkflowProgress(scanBuffer(sid, SCAN_LINES));
      result.authority = 'local';
      // Once an unsettled Hub snapshot becomes stale, the local freeze/miss
      // heuristic is the last safety net. Require output-idle in that case so
      // an actively producing session is never closed by a stale browser view.
      const allowFallbackSettle = !hubEntry || !!sessions.get(sid)?.output_idle;
      // 走行中である肯定的証拠。これが真の間は「完了」を生成も維持もしない。
      const authoritativeRunning = result.live || result.waitingDynamic > 0;
      if (result.detected) {
        recordWorkflowDoneLabels(sid, result.phases);
        missCounts.set(sid, 0);
        const sig = sigOf(result);
        const prev = snapshots.get(sid);
        // 別 Workflow（sig 変化）になったら dismiss を解除して再表示する。
        // ただしソース切替（hub → local fallback）の sig 差は同一 Workflow の
        // 書式差なので dismiss を引き継ぐ（F3・applyHubSnapshot 側と対称）。
        const keepDismissed = !!(prev && prev.dismissed && (prev.sig === sig || prev.source !== 'local'));
        // 走行中の権威的証拠があるときは settle を強制解除し、freeze/miss カウンタもリセット。
        let settled = !result.running && allowFallbackSettle;
        if (authoritativeRunning) {
          settled = false;
          freezeCounts.set(sid, 0);
          missCounts.set(sid, 0);
          lastFrameSig.set(sid, result.frameSig);
        } else if (result.running) {
          const sameFrame = lastFrameSig.get(sid) === result.frameSig;
          const frozen = (freezeCounts.get(sid) || 0) + (sameFrame ? 1 : 0);
          freezeCounts.set(sid, sameFrame ? frozen : 0);
          lastFrameSig.set(sid, result.frameSig);
          if (allowFallbackSettle && frozen >= SETTLE_MISS_LIMIT) settled = true;
        } else {
          freezeCounts.set(sid, 0);
          lastFrameSig.set(sid, result.frameSig);
        }
        // sticky-settle は走行証拠が無い同一 Workflow にだけ適用する。
        if (!authoritativeRunning && prev && prev.sig === sig && prev.settled) settled = true;
        snapshots.set(sid, { result, settled, dismissed: keepDismissed, sig, source: 'local' });
      } else {
        // 未検出。完了スナップショットは振り返り用に保持し続ける（消さない）。
        const prev = snapshots.get(sid);
        if (authoritativeRunning && prev) {
          prev.settled = false;
          missCounts.set(sid, 0);
          freezeCounts.set(sid, 0);
        } else if (prev && !prev.settled && allowFallbackSettle) {
          const miss = (missCounts.get(sid) || 0) + 1;
          missCounts.set(sid, miss);
          const limit = prev.result.live ? LIVE_SETTLE_MISS_LIMIT : SETTLE_MISS_LIMIT;
          if (miss >= limit) prev.settled = true;
        }
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

/** 今のセッションのサブエージェントの木を、描画用の行へ組み立てて返す。木が無ければ null。 */
function activeSubagentView(now: number): SubagentTreeView | null {
  const sid = activeSessionId;
  if (sid === null) return null;
  const entry = getSubagentTreeEntry(sid);
  if (!entry) return null;
  return buildSubagentTreeRows(entry.tree, now);
}

// hub ソースの完了判定は Hub の Settled のみを権威とする。カウントが完了形でも
// Hub が意図的に Settled=false を保持する窓（journal 紐付け待ち・10 秒静止待ち）
// があり、!running への OR フォールバックはその窓で「完了」を先走らせる
// （敵対レビュー 2026-08-05 F4）。ローカル fallback は従来どおり凍結ヒューリス
// ティック由来の !running も完了として扱う。
function snapshotDone(snap: WfSnapshot): boolean {
  if (snap.source === 'hub') return snap.settled;
  return snap.settled || !snap.result.running;
}

function formatElapsed(seconds: number, compact = false): string {
  const sec = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (compact) {
    if (h > 0) return `${h}h${m > 0 ? ` ${m}m` : ''}`;
    if (m > 0) return `${m}m`;
    return `${s}s`;
  }
  const parts: string[] = [];
  if (h > 0) parts.push(`${h}h`);
  if (m > 0 || h > 0) parts.push(`${m}m`);
  parts.push(`${s}s`);
  return parts.join(' ');
}

// 1 行表示（直近ツール等）へ入れる短縮テキスト。空白畳み込み + 上限文字数で丸める。
function clampForLine(raw: string, max = 100): string {
  const s = String(raw || '').replace(/\s+/g, ' ').trim();
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}

function formatTokenCount(n: number): string {
  if (!(n > 0)) return '';
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
  return String(n);
}

// tasks output 由来の 1 エージェント分メトリクス行（モデル・所要時間・トークン数・
// ツール呼び出し数）。値が無い項目は落として ' · ' 連結する。
function formatAgentDetailMetrics(detail: WfAgentDetail): string {
  const parts: string[] = [];
  if (detail.model) parts.push(detail.model);
  if (detail.duration_ms && detail.duration_ms > 0) {
    parts.push(formatElapsed(detail.duration_ms / 1000, true));
  }
  const tok = formatTokenCount(detail.tokens || 0);
  if (tok) parts.push(t('wf_detail_tokens', { n: tok }));
  if (detail.tool_calls && detail.tool_calls > 0) {
    parts.push(t('wf_detail_tool_calls', { n: detail.tool_calls }));
  }
  return parts.join(' · ');
}

// Workflow が検出されているときの表示文言（既存ロジックをそのまま関数化しただけ）。
function workflowPillText(snap: WfSnapshot, done: boolean): string {
  const elapsed = snap.result.elapsedSec && snap.result.elapsedSec > 0
    ? formatElapsed(snap.result.elapsedSec, true)
    : '';
  if (done) return t('wf_progress_pill_done', { n: snap.result.totalCount });
  if (snap.result.totalCount === 0 && snap.result.waitingDynamic > 0) {
    return t('wf_progress_pill_waiting', { n: snap.result.waitingDynamic });
  }
  if (elapsed) {
    return t('wf_progress_pill_live_elapsed', {
      done: snap.result.doneCount,
      total: snap.result.totalCount,
      elapsed,
    });
  }
  if (snap.result.live) {
    // 背景実行（N/M agents done）は done/total で下のステータス行と表示を揃える。
    return t('wf_progress_pill_live', { done: snap.result.doneCount, total: snap.result.totalCount });
  }
  return t('wf_progress_pill_running', { n: snap.result.runningCount });
}

// Workflow が無いときの、サブエージェントの木だけのチップ文言。
// 「{n} 実行中 · 完了 {n} · {いちばん古い走行中の子の経過時間}」（mockup S-01）。
// 走行中が 0 なら経過時間を出さず、各セグメントは値が 0 のものを落とす。
function subagentPillText(view: SubagentTreeView): string {
  const { running, done, failed } = view.summary;
  const parts: string[] = [];
  if (running > 0) parts.push(t('subagent_tree_pill_running_seg', { n: running }));
  if (done > 0) parts.push(t('subagent_tree_pill_done_seg', { n: done }));
  if (failed > 0) parts.push(t('subagent_tree_pill_failed_seg', { n: failed }));
  if (running > 0) {
    const oldest = view.rows.reduce(
      (max, row) => (row.state === 'running' && row.elapsedSec !== undefined ? Math.max(max, row.elapsedSec) : max),
      0,
    );
    if (oldest > 0) parts.push(formatElapsed(oldest, true));
  }
  const body = parts.join(' · ');
  return body ? t('subagent_tree_pill', { body }) : t('subagent_tree_modal_title');
}

// チップの DOM 更新は Workflow / サブエージェントの木で共通（記号・色・文言の差し替えだけ）。
function applyPillModel(pill: HTMLElement, done: boolean, text: string, title: string): void {
  pill.hidden = false;
  pill.classList.toggle('wf-done', done);
  // 走行中は回る輪、終わったらチェック。記号はモーダルの各エージェント行と同じものを使う。
  const spinnerEl = pill.querySelector('.wf-pill-spinner') as HTMLElement | null;
  if (spinnerEl) {
    const kind = done ? 'check' : 'ring';
    spinnerEl.classList.toggle('wf-running', !done);
    if (spinnerEl.dataset.iconKind !== kind) {
      spinnerEl.dataset.iconKind = kind;
      spinnerEl.innerHTML = stateIconSvgHtml(kind);
    }
  }
  const textEl = pill.querySelector('.wf-pill-text') as HTMLElement | null;
  if (textEl) textEl.textContent = text;
  const openBtn = pill.querySelector('.wf-pill-open') as HTMLElement | null;
  if (openBtn) openBtn.title = title;
}

function renderPill(): void {
  const pill = ensurePill();
  const sid = activeSessionId;
  const snap = activeSnapshot();
  const workflowVisible = !!snap && !snap.dismissed && snap.result.detected &&
    (snap.result.totalCount > 0 || snap.result.waitingDynamic > 0);

  if (workflowVisible) {
    const done = snapshotDone(snap!);
    applyPillModel(pill, done, workflowPillText(snap!, done), done ? t('wf_progress_done') : t('wf_progress_running'));
    return;
  }

  // Workflow が無い。サブエージェントの木があればそちらでチップを出す（親 plan 方針7）。
  const view = sid !== null ? activeSubagentView(Date.now()) : null;
  if (!view || view.rows.length === 0) {
    // 木が空＝次のまとまりの始まり。✕ で閉じた記憶も一緒に手放す。
    if (sid !== null) subagentDismissed.delete(sid);
    pill.hidden = true;
    if (modalOpen) closeWorkflowModal();
    return;
  }
  if (sid !== null && subagentDismissed.has(sid)) {
    pill.hidden = true;
    if (modalOpen) closeWorkflowModal();
    return;
  }
  const done = view.summary.running === 0;
  applyPillModel(
    pill,
    done,
    subagentPillText(view),
    done ? t('subagent_tree_pill_title_done') : t('subagent_tree_pill_title_running'),
  );
}

function dismissActive(ev?: Event): void {
  ev?.stopPropagation();
  // 表示中のピルが Workflow・サブエージェントのどちらでも、✕ は「今見えているもの」を
  // まとめて閉じる（片方だけ閉じると、閉じた直後にもう片方のチップへ入れ替わって見える）。
  const snap = activeSnapshot();
  if (snap) snap.dismissed = true;
  const sid = activeSessionId;
  if (sid !== null) subagentDismissed.add(sid);
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
  overlay.classList.add('aac-wheel-overlay');

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

// 記号は 4 状態とも共有の SVG（stateIconSvgHtml）で描く。✓ / ✗ / ○ のグリフを混ぜると
// 状態ごとにフォールバック先のフォントが変わり、送り幅も墨の位置も揃わない
// （session-list.ts の STATE_ICON_SVG 冒頭に実測値）。
const WF_STATE_ICON_KIND: Record<string, string> = {
  running: 'ring',
  done: 'check',
  failed: 'cross',
  pending: 'dot',
};

function stateIcon(state: WfAgentState): HTMLElement {
  const span = document.createElement('span');
  span.className = 'wf-agent-icon wf-state-' + state;
  // 記号を SVG にしたぶん、読み上げ用の状態名は属性で持たせる（✓ / ✗ / ○ の文字を
  // 置いていた頃は本文として読めていた）。
  const label = t('wf_state_' + state);
  span.setAttribute('role', 'img');
  span.setAttribute('aria-label', label);
  span.title = label;
  span.innerHTML = stateIconSvgHtml(WF_STATE_ICON_KIND[state] || 'dot');
  return span;
}

// サブエージェントの木は状態が 3 + 1（running/done/failed/unknown）で、Workflow の
// pending には対応しない。記号・色は「新しい丸を作らない」の規約どおり Workflow の
// バケツをそのまま借りる（unknown だけ、色バケツは pending＝灰色の点を流用しつつ、
// 読み上げ文言は「状態不明」で別にする）。
const SUBAGENT_ICON_KIND: Record<SubagentRowState, string> = {
  running: 'ring',
  done: 'check',
  failed: 'cross',
  unknown: 'dot',
};

function subagentStateIcon(state: SubagentRowState): HTMLElement {
  const span = document.createElement('span');
  const cssState = state === 'unknown' ? 'pending' : state;
  span.className = 'wf-agent-icon wf-state-' + cssState;
  const label = t('subagent_tree_state_' + state);
  span.setAttribute('role', 'img');
  span.setAttribute('aria-label', label);
  span.title = label;
  span.innerHTML = stateIconSvgHtml(SUBAGENT_ICON_KIND[state]);
  return span;
}

// 見出し横の集計（モーダル上部のカウント欄・Workflow 併記時のサブエージェント節見出し
// で共用）。「実行中 n · 完了 n」に、失敗があれば「失敗 n」を足す（値が 0 の区分は出さない）。
function subagentCountsText(summary: SubagentTreeSummary): string {
  const parts: string[] = [];
  if (summary.running > 0) parts.push(t('subagent_tree_header_running', { n: summary.running }));
  if (summary.done > 0) parts.push(t('subagent_tree_header_done', { n: summary.done }));
  if (summary.failed > 0) parts.push(t('subagent_tree_header_failed', { n: summary.failed }));
  return parts.join(' · ');
}

// 1 行分の右側テキスト（経過時間 / 失敗ラベル）。「最後の動き」は別枠（呼び出し側で
// 付け足す）なので、ここには含めない。
function subagentRowTimeText(row: SubagentTreeRow): string {
  const parts: string[] = [];
  if (row.state === 'failed') parts.push(t('subagent_tree_failed_label'));
  if (row.elapsedSec !== undefined) parts.push(formatElapsed(row.elapsedSec, false));
  return parts.join(' · ');
}

function subagentIndentPx(depth: number): string {
  return (Math.max(1, depth) - 1) * 18 + 'px';
}

// 走行中で今のツールの行が出る子かどうか（buildSubagentToolEl が実際に行を返す条件と
// 常にそろえる）。C11: この場合だけ、種別・モデル（row.detail）を名前の行ではなく
// ツールの行の右端に出す（完成イメージ S-02。560px のモーダルで名前を切らないため）。
function subagentHasToolLine(row: SubagentTreeRow): boolean {
  return row.state === 'running' && !!row.tool;
}

function buildSubagentRowEl(row: SubagentTreeRow): HTMLElement {
  const el = document.createElement('div');
  const settledClass =
    row.state === 'done' ? ' wf-subagent-row-done' : row.state === 'failed' ? ' wf-subagent-row-failed' : '';
  el.className = 'wf-agent-row wf-agent-' + row.state + settledClass;
  el.style.marginLeft = subagentIndentPx(row.depth);
  el.appendChild(subagentStateIcon(row.state));

  const lbl = document.createElement('span');
  lbl.className = 'wf-agent-label';
  lbl.textContent = row.name || t('subagent_tree_unnamed');
  el.appendChild(lbl);

  // ツールの行がある子は、種別・モデルをそちら（buildSubagentToolEl）へ移す。
  if (row.detail && !subagentHasToolLine(row)) {
    const detail = document.createElement('span');
    detail.className = 'wf-subagent-detail';
    detail.textContent = row.detail;
    el.appendChild(detail);
  }

  const timeText = subagentRowTimeText(row);
  if (timeText || row.lastActivitySec !== undefined) {
    const mt = document.createElement('span');
    mt.className = 'wf-agent-metrics';
    if (timeText) mt.appendChild(document.createTextNode(timeText));
    if (row.lastActivitySec !== undefined) {
      if (timeText) mt.appendChild(document.createTextNode(' · '));
      const age = document.createElement('span');
      age.className = 'wf-subagent-age';
      age.textContent = t('subagent_tree_last_activity', { n: row.lastActivitySec });
      mt.appendChild(age);
    }
    el.appendChild(mt);
  }
  return el;
}

// 走行中で今のツールが分かる行にだけ、既存の Workflow 詳細と同じ文面を足す
// （子 plan C1 の「実装時の確定」: 生の値のまま返すので、i18n は wf_detail_last_tool を流用）。
// C11: 種別・モデル（row.detail）が付くときは、行の右端（.wf-subagent-detail）へ足す
// （完成イメージ S-02 の `.tool .tx` / `.tool .sub`）。名前の行 (buildSubagentRowEl) 側では
// 出さないので、二重には出ない。
function buildSubagentToolEl(row: SubagentTreeRow): HTMLElement | null {
  if (!subagentHasToolLine(row)) return null;
  const el = document.createElement('div');
  el.className = 'wf-subagent-tool';
  el.style.marginLeft = subagentIndentPx(row.depth);

  const text = document.createElement('span');
  text.className = 'wf-subagent-tool-text';
  // Grok の記録にはツールの対象の欄が無く、要約は常に空になる（親 plan §C9）。空のときに
  // 「read_file —」と区切りだけが残らないよう、名前だけの文面に切り替える。
  const summary = clampForLine(row.tool?.summary || '');
  text.textContent = summary
    ? t('wf_detail_last_tool', { name: row.tool?.name || '', summary })
    : t('wf_detail_last_tool_name_only', { name: row.tool?.name || '' });
  el.appendChild(text);

  if (row.detail) {
    const detail = document.createElement('span');
    detail.className = 'wf-subagent-detail';
    detail.textContent = row.detail;
    el.appendChild(detail);
  }
  return el;
}

// Workflow の節の下（併記時）／モーダル本文の先頭（単独時）に足す「サブエージェント」節。
// headingSub は S-02 の「最終更新 N 秒前」・S-03 の「実行中 n」のどちらか（無ければ省略）。
function buildSubagentSectionEl(view: SubagentTreeView, headingText: string, headingSub?: string): HTMLElement {
  const section = document.createElement('section');
  section.className = 'wf-subagent-section';

  const heading = document.createElement('div');
  heading.className = 'wf-subagent-heading';
  heading.textContent = headingText;
  if (headingSub) {
    const sub = document.createElement('span');
    sub.className = 'wf-subagent-heading-sub';
    sub.textContent = headingSub;
    heading.appendChild(sub);
  }
  section.appendChild(heading);

  const rows = document.createElement('div');
  rows.className = 'wf-agent-list';
  for (const row of view.rows) {
    rows.appendChild(buildSubagentRowEl(row));
    const toolEl = buildSubagentToolEl(row);
    if (toolEl) rows.appendChild(toolEl);
  }
  section.appendChild(rows);

  if (view.summary.omitted > 0) {
    const omit = document.createElement('div');
    omit.className = 'wf-subagent-omitted';
    omit.textContent = t('subagent_tree_omitted', { n: view.summary.omitted });
    section.appendChild(omit);
  }
  return section;
}

function renderModalBody(): void {
  const overlay = document.getElementById('workflow-modal');
  if (!overlay) return;
  const counts = overlay.querySelector('.wf-modal-counts') as HTMLElement | null;
  const titleEl = overlay.querySelector('.wf-modal-title') as HTMLElement | null;
  const body = overlay.querySelector('.wf-modal-body') as HTMLElement | null;
  if (!body) return;

  const snap = activeSnapshot();
  const r = snap && !snap.dismissed ? snap.result : null;
  const workflowDetected = !!r && r.detected && (r.totalCount > 0 || r.waitingDynamic > 0);

  const sidForTree = activeSessionId;
  const now = Date.now();
  const subagentView = sidForTree !== null ? activeSubagentView(now) : null;
  // 単独表示（Workflow 無し）のときだけ ✕ の記憶を見る。Workflow と併記のときは
  // Workflow 側の dismissed が既にモーダルごと閉じている（呼び出し元 renderPill/
  // dismissActive が両方を一緒に閉じる）ので、ここで二重に隠さない。
  const subagentSuppressed = !workflowDetected && sidForTree !== null && subagentDismissed.has(sidForTree);
  const hasSubagent = !!subagentView && subagentView.rows.length > 0 && !subagentSuppressed;

  // 解釈不能（検出が外れた / フォーマット不一致）かつサブエージェントの木も無い:
  // クラッシュさせず案内を出す。
  if (!workflowDetected && !hasSubagent) {
    if (titleEl) titleEl.textContent = t('wf_progress_title');
    if (counts) counts.textContent = '';
    body.innerHTML = '';
    const empty = document.createElement('div');
    empty.className = 'wf-modal-empty';
    empty.textContent = t('wf_progress_unparsed');
    body.appendChild(empty);
    return;
  }

  body.innerHTML = '';

  if (!workflowDetected) {
    // Workflow 無し・サブエージェントの木だけ。モーダル上部はこちらの集計にする。
    if (titleEl) titleEl.textContent = t('subagent_tree_modal_title');
    if (counts) counts.textContent = subagentCountsText(subagentView!.summary);
    const provider = String(getSubagentTreeEntry(sidForTree!)?.tree.provider || '').toUpperCase();
    let headingSub: string | undefined;
    if (subagentView!.summary.running > 0) {
      const receivedAt = getSubagentTreeEntry(sidForTree!)?.receivedAt;
      if (receivedAt !== undefined) {
        headingSub = t('subagent_tree_section_updated', { n: Math.max(0, Math.round((now - receivedAt) / 1000)) });
      }
    }
    const headingText = provider
      ? t('subagent_tree_section_standalone', { provider })
      : t('subagent_tree_modal_title');
    body.appendChild(buildSubagentSectionEl(subagentView!, headingText, headingSub));
    return;
  }

  if (titleEl) titleEl.textContent = t('wf_progress_title');
  const done = snapshotDone(snap!);
  // done（settle 済み / 走行終了）のときは表示を完了側へ倒す。
  // settle はフレーム凍結ヒューリスティックで確定するため、生パース結果の
  // running グリフ・件数・percent はまだ走行中のまま残っている。ピル（バッジ）と
  // 整合させ、走行中件数を 0・残りを done・バーを 100% として描画する。
  const dispRunning = done ? 0 : r.runningCount;
  const dispDone = done ? Math.max(r.doneCount, r.totalCount - r.failedCount) : r.doneCount;
  const dispPercent = done ? 100 : r.percent;
  // Hub が tasks output（Claude Code Workflow タスク出力）から taskId を解決できた
  // ときだけ true。true の間は各 agent 行へ詳細（モデル・所要時間・直近ツール・
  // 結果プレビュー）を追加描画し、完了台帳（下部の wf-completed-ledger）は
  // 詳細セクションが役割を包含するため出さない（plan §C3）。
  const detailMode = r.taskDetailSource === 'task-output';
  if (counts) {
    const parts: string[] = [];
    if (r.totalCount > 0) {
      parts.push(
        t('wf_progress_counts_running', { n: dispRunning }),
        t('wf_progress_counts_done', { n: dispDone }),
      );
      if (r.failedCount > 0) parts.push(t('wf_progress_counts_failed', { n: r.failedCount }));
    }
    if (r.elapsedSec && r.elapsedSec > 0) parts.push(formatElapsed(r.elapsedSec));
    if (r.tokensRaw) parts.push(r.tokensRaw);
    counts.textContent = parts.join(' · ');
  }

  // 進捗バー（percent があれば。done なら 100% 固定）。
  if (dispPercent !== null && r.totalCount > 0) {
    const barWrap = document.createElement('div');
    barWrap.className = 'wf-progress-bar';
    const fill = document.createElement('div');
    fill.className = 'wf-progress-fill';
    fill.style.width = Math.max(0, Math.min(100, dispPercent)) + '%';
    if (done) fill.classList.add('wf-progress-fill-done');
    barWrap.appendChild(fill);
    const pct = document.createElement('span');
    pct.className = 'wf-progress-pct';
    pct.textContent = dispPercent + '%';
    barWrap.appendChild(pct);
    body.appendChild(barWrap);
  }

  if (r.name) {
    const nameEl = document.createElement('div');
    nameEl.className = 'wf-modal-name';
    nameEl.textContent = r.name;
    body.appendChild(nameEl);
  }

  if (r.waitingDynamic > 0) {
    const waiting = document.createElement('div');
    waiting.className = 'wf-waiting-dynamic';
    waiting.textContent = t('wf_progress_waiting_dynamic', { n: r.waitingDynamic });
    body.appendChild(waiting);
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
      // done なら残った running グリフ（凍結スピナー）を完了アイコンへ倒す。
      const effState: WfAgentState =
        done && agent.state === 'running' ? 'done' : agent.state;
      const row = document.createElement('div');
      row.className = 'wf-agent-row wf-agent-' + effState;
      row.appendChild(stateIcon(effState));
      const lbl = document.createElement('span');
      lbl.className = 'wf-agent-label';
      lbl.textContent = agent.label;
      // promptPreview はラベルの代替にはしない（label が既に人間可読）。
      // ホバー時 tooltip としてだけ補足で出す（plan §C3）。
      if (detailMode && agent.detail?.prompt_preview) {
        lbl.title = agent.detail.prompt_preview;
      }
      row.appendChild(lbl);
      if (agent.metrics) {
        const metrics = document.createElement('span');
        metrics.className = 'wf-agent-metrics';
        metrics.textContent = agent.metrics;
        row.appendChild(metrics);
      }
      if (detailMode && agent.detail) {
        const detailMetrics = formatAgentDetailMetrics(agent.detail);
        if (detailMetrics) {
          const metrics = document.createElement('span');
          metrics.className = 'wf-agent-metrics wf-agent-detail-metrics';
          metrics.textContent = detailMetrics;
          row.appendChild(metrics);
        }
      }
      list.appendChild(row);

      if (detailMode && agent.detail) {
        if (agent.state === 'running' && (agent.detail.last_tool_name || agent.detail.last_tool_summary)) {
          // 走行中エージェント: 直近ツールを 1 行で出す（結果プレビューは完了後のみ）。
          const lastTool = document.createElement('div');
          lastTool.className = 'wf-agent-last-tool';
          lastTool.textContent = t('wf_detail_last_tool', {
            name: agent.detail.last_tool_name || '',
            summary: clampForLine(agent.detail.last_tool_summary || ''),
          });
          list.appendChild(lastTool);
        } else if (agent.state === 'done' && agent.detail.result_preview) {
          // 完了エージェント: 結果プレビューは折りたたみ（既定は閉じる＝一覧性優先）。
          const details = document.createElement('details');
          details.className = 'wf-agent-result';
          const summaryEl = document.createElement('summary');
          summaryEl.textContent = t('wf_detail_result_summary');
          details.appendChild(summaryEl);
          const resultBody = document.createElement('div');
          resultBody.className = 'wf-agent-result-body';
          resultBody.textContent = agent.detail.result_preview;
          details.appendChild(resultBody);
          list.appendChild(details);
        }
      }
    }
    phaseEl.appendChild(list);
    body.appendChild(phaseEl);
  }


  const sid = activeSessionId;
  // detailMode（tasks output 由来の詳細セクション）が有効なセッションでは、
  // 上の agent 行が既に完了エージェントの結果を折りたたみで持っているため、
  // 完了台帳（VT/journal のみで label しか分からない旧来表示）は出さない。
  if (sid !== null && !detailMode) {
    const ledger = getWorkflowLedger(sid, r.doneCount);
    if (ledger.labels.length > 0 || ledger.otherCount > 0) {
      const section = document.createElement('section');
      section.className = 'wf-completed-ledger';
      const heading = document.createElement('div');
      heading.className = 'wf-completed-title';
      heading.textContent = t('wf_progress_completed_seen');
      section.appendChild(heading);
      const list = document.createElement('div');
      list.className = 'wf-completed-list';
      for (const label of ledger.labels) {
        const row = document.createElement('div');
        row.className = 'wf-completed-row';
        row.textContent = `✓ ${label}`;
        list.appendChild(row);
      }
      if (ledger.otherCount > 0) {
        const other = document.createElement('div');
        other.className = 'wf-completed-other';
        other.textContent = t('wf_progress_completed_other', { n: ledger.otherCount });
        list.appendChild(other);
      }
      section.appendChild(list);
      body.appendChild(section);
    }
  }

  // Workflow の節の下に、サブエージェントの節を足す（S-03）。Workflow の子は上で
  // 数えているので、ここで二重に出さない（親 plan 方針6・Hub 側が別フィールドで
  // 送ってくる時点で二重化は起きない）。
  if (hasSubagent) {
    const headingSub = subagentCountsText(subagentView!.summary);
    body.appendChild(buildSubagentSectionEl(subagentView!, t('subagent_tree_modal_title'), headingSub));
  }
}

// ── 外部 API ─────────────────────────────────────────────────────────────────

/** Hub の workflow_progress を全セッション分保持する。 */
export function receiveWorkflowProgress(
  sessionId: number,
  progress: HubWorkflowProgress,
  receivedAt = Date.now(),
): void {
  if (!Number.isFinite(sessionId) || sessionId <= 0 || !progress) return;
  setHubWorkflowProgress(sessionId, progress, receivedAt);
  applyHubSnapshot(sessionId, receivedAt);
  if (sessionId === activeSessionId) {
    renderPill();
    if (modalOpen) renderModalBody();
  }
}

/** session_removed 時に保持スナップショットをクリアする（メモリ保持はセッション生存中のみ）。 */
export function removeWorkflowSnapshot(sessionId: number): void {
  snapshots.delete(sessionId);
  missCounts.delete(sessionId);
  freezeCounts.delete(sessionId);
  lastFrameSig.delete(sessionId);
  subagentDismissed.delete(sessionId);
  removeWorkflowStore(sessionId);
  if (sessionId === activeSessionId) renderPill();
}

/**
 * ws-client が subagent_tree を保持した直後に呼ぶ。800ms の poll を待たず、今見て
 * いるセッションのものならチップ/モーダルをすぐ描き直す（receiveWorkflowProgress
 * と同じ即時反映）。
 */
export function receiveSubagentTree(sessionId: number): void {
  if (!Number.isFinite(sessionId) || sessionId <= 0) return;
  if (sessionId === activeSessionId) {
    renderPill();
    if (modalOpen) renderModalBody();
  }
}

/** DOM 準備後に呼ぶ。ポーリングを開始し、Workflow 検出時のみピルを出す。 */
export function initWorkflowProgress(): void {
  ensurePill();
  if (pollTimer) return;
  pollTimer = setInterval(poll, POLL_MS);
}

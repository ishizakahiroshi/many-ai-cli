// 一時観測用（Codex ターミナルに空行が積もる / rows が発振する件の原因特定）。
//
// Web 側でしか分からない「どのレイアウト変化が pty_resize を誘発したか」を
// Hub の ~/.many-ai-cli/logs/debug/ui-log-YYYY-MM-DD.jsonl へ集める。
// DevTools を開かずに時系列を回収するための仕込みであり、原因が確定したら
// 本ファイルと /api/debug/ui-log ごと撤去する（撤去済みの batch-log と同じ扱い）。
import { apiFetch } from './util.js';

const FLUSH_DELAY_MS = 400;
const MAX_QUEUE = 500;
const MAX_BATCH = 200;

// 高さが変わると #terminal-area が縮む要素群。resize の犯人を名指しするために
// 「その瞬間の各要素の高さ」をまとめて 1 レコードへ入れる。
const TRACKED_ELEMENT_IDS = [
  'display-area',
  'terminal-wrapper',
  'terminal-area-wrapper',
  'terminal-area',
  'action-bar',
  'multi-question-banner',
  'approval-suppressed-banner',
  'stale-binary-banner',
  'terminal-live-status',
  'mobile-nudge-panel',
  'input-bar-outer',
  'input-bar',
  'input',
];

let queue: any[] = [];
let flushTimer: any = null;
let droppedSinceFlush = 0;
let enabled = true;

export function setUiDebugLogEnabled(on: boolean): void {
  enabled = on;
}

export function uiDebugLog(event: string, fields: Record<string, any> = {}): void {
  if (!enabled) return;
  if (queue.length >= MAX_QUEUE) {
    droppedSinceFlush++;
    return;
  }
  let ts = '';
  try { ts = new Date().toISOString(); } catch (_) { /* 取得不能でもイベントは残す */ }
  queue.push({ ts, event, ...fields });
  scheduleFlush();
}

// 要素の高さスナップショット。display:none / hidden は 0、要素が無い場合は -1。
// getClientRects().length === 0 で非表示を判定する（getComputedStyle より軽い）。
export function layoutSnapshot(): Record<string, number> {
  const snap: Record<string, number> = {};
  for (const id of TRACKED_ELEMENT_IDS) {
    const el = document.getElementById(id);
    if (!el) { snap[id] = -1; continue; }
    snap[id] = el.getClientRects().length === 0 ? 0 : Math.round(el.getBoundingClientRect().height);
  }
  const bar = document.getElementById('action-bar');
  snap['action-bar.visible'] = bar && bar.classList.contains('visible') ? 1 : 0;
  return snap;
}

function scheduleFlush(): void {
  if (flushTimer) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    void flushUiDebugLog();
  }, FLUSH_DELAY_MS);
}

export async function flushUiDebugLog(): Promise<void> {
  if (!queue.length) return;
  const events = queue.splice(0, MAX_BATCH);
  if (droppedSinceFlush > 0) {
    events.push({ ts: new Date().toISOString(), event: 'ui_debug_log_dropped', count: droppedSinceFlush });
    droppedSinceFlush = 0;
  }
  try {
    await apiFetch('/api/debug/ui-log', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ events }),
      keepalive: true,
    });
  } catch (_) { /* 観測用。送信失敗は本体動作に影響させない */ }
  if (queue.length) scheduleFlush();
}

if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => { void flushUiDebugLog(); });
}

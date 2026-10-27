// scroll-follow-hud.ts — 一時観測専用。端末が最下部へ追従しない不具合の観測。
//
// 症状が出た瞬間をスクリーンショットで読めるよう、画面の隅に小さな表示を出す
// （既定は非表示。Ctrl+Alt+H で表示/非表示）。
// DevTools もログファイルも使わない。保存・送信はせず、数値と分岐名だけを画面に出す
// （端末の本文・パス・入力は読まない）。
//
// 表示の読み方:
//   auto      追従フラグ（autoScroll）。false だと出力が増えても最下部へ戻らない
//   vY / bY   viewportY（表示の先頭行）と baseY（最下部のときの先頭行）。gap = bY - vY
//   miss      追従 ON のまま出力を書いたのに、scrollToBottom の後も最下部でなかった回数
//   held      追従 OFF のまま出力が来た回数
//   events    onScroll でフラグがどう決まったか（最新が下）
//             at-bottom=最下部に居るので ON / manual-off=手動操作の直後に最下部を外れたので OFF
//             snap=手動でないのに外れたので最下部へ戻した / hold=OFF のまま何もしなかった
//             wheel=ホイール操作の結果

import { registerProbeSink } from './probe.js';
import { activeSessionId, terminals } from '../app/state.js';

interface FollowEvent {
  at: number;
  branch: string;
  before: boolean;
  after: boolean;
  vy: number;
  by: number;
  sinceManualMs: number;
}

const EVENT_LIMIT = 60;
const SHOWN_RUNS = 10;
const events: FollowEvent[] = [];
const counters = { outputs: 0, miss: 0, held: 0 };
let lastOutputAt = 0;
let lastMiss: { at: number; vy: number; by: number } | null = null;

registerProbeSink('scroll.follow', (_channel, fields) => {
  try {
    if (fields.sessionId !== activeSessionId) return;
    events.push({
      at: Date.now(),
      branch: String(fields.branch),
      before: !!fields.autoBefore,
      after: !!fields.autoAfter,
      vy: Number(fields.viewportY),
      by: Number(fields.baseY),
      sinceManualMs: Number(fields.sinceManualMs),
    });
    if (events.length > EVENT_LIMIT) events.shift();
  } catch { /* observation only */ }
});

registerProbeSink('scroll.output', (_channel, fields) => {
  try {
    if (fields.sessionId !== activeSessionId) return;
    counters.outputs++;
    lastOutputAt = Date.now();
    if (!fields.autoScroll) {
      counters.held++;
    } else if (Number(fields.viewportY) < Number(fields.baseY)) {
      counters.miss++;
      lastMiss = { at: Date.now(), vy: Number(fields.viewportY), by: Number(fields.baseY) };
    }
  } catch { /* observation only */ }
});

function ago(at: number): string {
  if (!at) return '-';
  const s = (Date.now() - at) / 1000;
  return s < 100 ? `${s.toFixed(1)}s` : `${Math.round(s)}s`;
}

function liveLine(): string {
  const t: any = activeSessionId == null ? null : terminals.get(activeSessionId);
  const buf = t?.term?.buffer?.active;
  if (!t || !buf) return 'no active terminal';
  const rows = t.term.rows;
  return `auto=${t.autoScroll} vY=${buf.viewportY} bY=${buf.baseY} gap=${buf.baseY - buf.viewportY} len=${buf.length} rows=${rows} buf=${buf.type}`;
}

function runsText(): string {
  // 同じ分岐・同じフラグ変化が続いたものは 1 行にまとめる。
  const runs: { key: string; ev: FollowEvent; count: number }[] = [];
  for (const ev of events) {
    const key = `${ev.branch}:${ev.before}>${ev.after}`;
    const last = runs[runs.length - 1];
    if (last && last.key === key) {
      last.count++;
      last.ev = ev;
    } else {
      runs.push({ key, ev, count: 1 });
    }
  }
  return runs.slice(-SHOWN_RUNS).map(({ ev, count }) =>
    `${ago(ev.at).padStart(6)} ${ev.branch.padEnd(10)} ${ev.before}>${ev.after} vY=${ev.vy} bY=${ev.by} man=${ev.sinceManualMs}ms${count > 1 ? ` x${count}` : ''}`,
  ).join('\n');
}

function render(el: HTMLElement): void {
  const missText = lastMiss ? ` last-miss ${ago(lastMiss.at)} vY=${lastMiss.vy} bY=${lastMiss.by}` : '';
  el.textContent = [
    'scroll-follow (debug)',
    liveLine(),
    `outputs=${counters.outputs} (last ${ago(lastOutputAt)}) miss=${counters.miss} held=${counters.held}${missText}`,
    'events:',
    runsText() || '(none)',
  ].join('\n');
}

// 表示は既定で OFF。Ctrl+Alt+H で ON/OFF を切り替え、状態は localStorage に残す
// （収集は常に動くので、ON にした瞬間から直近の分岐が読める）。
const FLAG_KEY = 'mai.debugHud';

function mount(): void {
  const el = document.createElement('div');
  el.id = 'mai-scroll-follow-hud';
  el.style.cssText = [
    'position:fixed', 'right:8px', 'bottom:8px', 'z-index:2147483000',
    'max-width:560px', 'padding:6px 8px', 'border-radius:4px',
    'background:rgba(0,0,0,.82)', 'color:#7CFC00',
    'font:11px/1.35 Consolas,monospace', 'white-space:pre', 'pointer-events:none',
  ].join(';');
  const enabled = (): boolean => {
    try { return localStorage.getItem(FLAG_KEY) === '1'; } catch { return false; }
  };
  const apply = (): void => {
    el.style.display = enabled() ? '' : 'none';
    if (enabled()) render(el);
  };
  document.body.appendChild(el);
  apply();
  setInterval(() => { if (enabled()) render(el); }, 300);
  document.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey && e.altKey && !e.shiftKey && e.code === 'KeyH')) return;
    try { localStorage.setItem(FLAG_KEY, enabled() ? '0' : '1'); } catch { /* observation only */ }
    apply();
  });
}

if (document.body) mount();
else document.addEventListener('DOMContentLoaded', mount, { once: true });

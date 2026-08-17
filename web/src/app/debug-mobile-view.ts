// debug-mobile-view.ts
// 一時観測: スマホ幅で mobile-terminal-lite のチャットが空のまま表示される症状の切り分け用。
// URL に ?mtldebug=1 を付けて開いたときだけ動作し、既定では 1 バイトも送らない。
// 送るのは行数・文字数・寸法・状態フラグだけで、ターミナル本文と入力テキストは含めない。
// 原因が確定したら撤去する（instrumentation.json の mobile-lite-empty）。

import { getMobileTranscriptMessages } from './mobile-transcript.js';
import { activeSessionId, sessions, terminals } from './state.js';
import { scanBuffer } from './terminal.js';
import { apiFetch } from './util.js';

const SAMPLE_INTERVAL_MS = 5000;
// 開いたまま放置しても増え続けないよう 5 分ぶんで自動停止する。
const MAX_SAMPLES = 60;

function enabled(): boolean {
  try {
    return new URLSearchParams(window.location.search).get('mtldebug') === '1';
  } catch {
    return false;
  }
}

let sent = 0;

function elemInfo(id: string, prefix: string, out: Record<string, unknown>): void {
  const el = document.getElementById(id);
  out[`${prefix}_exists`] = !!el;
  if (!el) return;
  const cs = window.getComputedStyle(el);
  out[`${prefix}_display`] = cs.display;
  out[`${prefix}_visibility`] = cs.visibility;
  out[`${prefix}_w`] = el.clientWidth;
  out[`${prefix}_h`] = el.clientHeight;
}

function collect(): Record<string, unknown> {
  const rec: Record<string, unknown> = {
    seq: sent,
    ua: String(navigator.userAgent || '').slice(0, 140),
    inner_w: window.innerWidth,
    inner_h: window.innerHeight,
    dpr: window.devicePixelRatio,
    mql_720: window.matchMedia('(max-width: 720px)').matches,
    mql_coarse: window.matchMedia('(pointer: coarse)').matches,
    body_home_view: document.body.classList.contains('mobile-home-view'),
    body_has_session: document.body.classList.contains('mobile-has-session'),
    session_count: sessions.size,
    terminal_count: terminals.size,
    active_session_id: activeSessionId,
  };
  elemInfo('terminal-area-wrapper', 'wrap', rec);
  elemInfo('mobile-terminal-lite', 'lite', rec);
  const lite = document.getElementById('mobile-terminal-lite');
  rec.lite_dom_msgs = lite ? lite.querySelectorAll('.mtl-msg').length : -1;

  const sid = activeSessionId;
  if (sid === null) return rec;
  rec.session_state = String(sessions.get(sid)?.state || '');
  // TODO(ts): TerminalEntry.term は state.ts 側で any 定義のため any のまま扱う。
  const entry: any = terminals.get(sid);
  rec.has_term_entry = !!entry;
  rec.has_term = !!entry?.term;
  if (entry?.term) {
    rec.cols = entry.term.cols;
    rec.rows = entry.term.rows;
    rec.term_has_element = !!entry.term.element;
    const buf = entry.term.buffer?.active;
    rec.buf_len = buf ? buf.length : -1;
    rec.buf_base_y = buf ? buf.baseY : -1;
    rec.buf_cursor_y = buf ? buf.cursorY : -1;
  }
  rec.pending_chunks = entry?.pendingChunks?.length ?? -1;
  const lines = scanBuffer(sid, 800);
  rec.scan_lines = lines.length;
  rec.scan_nonempty = lines.filter((l: string) => l.trim() !== '').length;
  rec.scan_chars = lines.join('\n').trim().length;
  rec.transcript_msgs = getMobileTranscriptMessages(sid).length;
  return rec;
}

function sample(): void {
  if (sent >= MAX_SAMPLES) return;
  sent++;
  let payload: Record<string, unknown>;
  try {
    payload = collect();
  } catch (e) {
    payload = { seq: sent, collect_error: String(e).slice(0, 200) };
  }
  void apiFetch('/api/debug/mobile-view', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  }).catch(() => {
    // 観測が本体の動作を止めないよう握り潰す。
  });
}

if (enabled()) {
  window.setTimeout(sample, 1500);
  window.setInterval(sample, SAMPLE_INTERVAL_MS);
}

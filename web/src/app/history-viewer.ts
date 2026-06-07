// 過去ログビューア: xterm の scrollback 上限より前の出力を、Hub の
// /api/session-log（生 PTY ログの範囲読み）でページ単位に遡って表示する。
// ライブターミナルには行を「前に差し込む」API が無いため、読み取り専用の
// 別 xterm をオーバーレイで重ねるページング方式を取る。
import { showToast, ti18n, token } from './util.js';
import { activeSessionId, sessions, terminals } from './state.js';
import { FONTSIZE_MAP, STORAGE_FONTSIZE_KEY } from './user-prefs.js';

// 1 ページに読む生ログのバイト数（サーバ側上限 512KB 以内）
const HV_CHUNK_BYTES = 128 * 1024;

// ページ描画前に除去・置換する制御シーケンス。
// ログ中間からの再生では絶対カーソル移動・画面消去が前後の文脈を失って
// ページ内容を上書き破壊するため、改行に置き換えて「流れ」として読めるようにする。
const HV_REWRITE_RE = /\x1b\[[0-9;]*[Hf]|\x1b\[2J|\x1b\[3J/g;
const HV_DROP_RE = /\x1b\[\?1049[hl]|\x1b\[\?2026[hl]|\[\/?ANY-AI-CLI\]/g;

let hvRoot: any = null;        // オーバーレイ要素
let hvOpenBtn: any = null;     // 上端到達時に出す「過去ログ」ボタン
let hvTerm: any = null;        // ビューア用 xterm
let hvFit: any = null;
let hvState: { sid: number; size: number; offset: number } | null = null;
let hvLoading = false;

function hvLabel(key: string, fallback: string): string {
  const v = ti18n(key);
  return (v && v !== key) ? v : fallback;
}

function ensureOpenBtn() {
  if (hvOpenBtn) return hvOpenBtn;
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return null;
  const btn = document.createElement('button');
  btn.id = 'history-viewer-open-btn';
  btn.type = 'button';
  btn.hidden = true;
  btn.addEventListener('click', () => {
    if (activeSessionId === null || activeSessionId === undefined) return;
    openHistoryViewer(activeSessionId);
  });
  wrapper.appendChild(btn);
  hvOpenBtn = btn;
  return btn;
}

// ライブターミナルがスクロールバック上端に到達したらボタンを表示する。
// terminal.ts の onScroll / attach 経路から呼ばれる。
export function updateHistoryHint(id: number) {
  if (id !== activeSessionId) return;
  const btn = ensureOpenBtn();
  if (!btn) return;
  const t = terminals.get(id);
  if (!t || !t.term || !t.term.buffer) { btn.hidden = true; return; }
  const buf = t.term.buffer.active;
  // alt buffer（TUI 全画面モード）は scrollback を持たないため対象外
  const atTopWithHistory = buf.type !== 'alternate'
    && buf.viewportY === 0
    && buf.length > t.term.rows;
  btn.hidden = !atTopWithHistory;
  if (!btn.hidden) {
    btn.textContent = hvLabel('hv_open_btn', '▲ 過去ログ');
    btn.title = hvLabel('hv_open_tooltip', 'スクロールバックより前をログから表示');
  }
}

export function hideHistoryHint() {
  if (hvOpenBtn) hvOpenBtn.hidden = true;
}

function ensureViewer() {
  if (hvRoot) return hvRoot;
  const wrapper = document.getElementById('terminal-area-wrapper');
  if (!wrapper) return null;
  const root = document.createElement('div');
  root.id = 'history-viewer';
  root.hidden = true;
  // document レベルの wheel 転送（terminal.ts）から除外し、ビューア xterm の
  // ネイティブスクロールを生かす
  root.setAttribute('data-wheel-native', '');

  const header = document.createElement('div');
  header.className = 'hv-header';
  const title = document.createElement('span');
  title.className = 'hv-title';
  const range = document.createElement('span');
  range.className = 'hv-range';
  const mkBtn = (cls: string, handler: () => void) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'hv-btn ' + cls;
    b.addEventListener('click', handler);
    return b;
  };
  const firstBtn = mkBtn('hv-first', () => hvGoto(0));
  const prevBtn = mkBtn('hv-prev', () => { if (hvState) hvGoto(Math.max(0, hvState.offset - HV_CHUNK_BYTES)); });
  const nextBtn = mkBtn('hv-next', () => { if (hvState) hvGoto(hvState.offset + HV_CHUNK_BYTES); });
  const lastBtn = mkBtn('hv-last', () => hvGoto(-1));
  const closeBtn = mkBtn('hv-close', closeHistoryViewer);
  header.appendChild(title);
  header.appendChild(range);
  header.appendChild(firstBtn);
  header.appendChild(prevBtn);
  header.appendChild(nextBtn);
  header.appendChild(lastBtn);
  header.appendChild(closeBtn);

  const body = document.createElement('div');
  body.className = 'hv-body';
  root.appendChild(header);
  root.appendChild(body);
  wrapper.appendChild(root);
  hvRoot = root;
  return root;
}

function hvApplyLabels() {
  if (!hvRoot) return;
  const set = (sel: string, key: string, fallback: string) => {
    const el = hvRoot.querySelector(sel);
    if (el) el.textContent = hvLabel(key, fallback);
  };
  set('.hv-title', 'hv_title', '過去ログ（読み取り専用）');
  set('.hv-first', 'hv_first', '⏮ 先頭');
  set('.hv-prev', 'hv_prev', '← 前');
  set('.hv-next', 'hv_next', '次 →');
  set('.hv-last', 'hv_last', '末尾 ⏭');
  set('.hv-close', 'hv_close', '✕ 閉じる');
}

function ensureViewerTerm() {
  if (hvTerm) return hvTerm;
  const body = hvRoot ? hvRoot.querySelector('.hv-body') : null;
  if (!body) return null;
  // ライブターミナル（terminal.ts）と同じ等幅フォント設定で読みやすさを揃える
  hvTerm = new Terminal({
    cursorBlink: false,
    scrollback: 20000,
    fontFamily: '"MS Gothic", "BIZ UDGothic", "BIZ UDゴシック", "Segoe UI Symbol", "Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace',
    fontSize: FONTSIZE_MAP[localStorage.getItem(STORAGE_FONTSIZE_KEY)] || 13,
    lineHeight: 1.25,
    theme: { background: '#0d1117', cursor: '#0d1117', cursorAccent: '#e6edf3' },
    disableStdin: true,
    allowProposedApi: true,
  });
  hvFit = new FitAddon.FitAddon();
  hvTerm.loadAddon(hvFit);
  hvTerm.open(body);
  return hvTerm;
}

function hvDecodeChunk(b64: string): string {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder('utf-8').decode(bytes);
}

function hvSanitize(text: string, offset: number): string {
  // ページ先頭の中途半端な行（前ページとの切れ目）は最初の改行まで捨てる
  if (offset > 0) {
    const nl = text.indexOf('\n');
    if (nl >= 0 && nl < text.length - 1) text = text.slice(nl + 1);
  }
  return text.replace(HV_REWRITE_RE, '\r\n').replace(HV_DROP_RE, '');
}

function hvUpdateRangeLabel() {
  if (!hvRoot || !hvState) return;
  const el = hvRoot.querySelector('.hv-range');
  if (!el) return;
  const from = hvState.offset;
  const to = Math.min(hvState.offset + HV_CHUNK_BYTES, hvState.size);
  const pct = hvState.size > 0 ? Math.round((to / hvState.size) * 100) : 100;
  el.textContent = `${Math.round(from / 1024)}–${Math.round(to / 1024)} KB / ${Math.round(hvState.size / 1024)} KB (${pct}%)`;
}

async function hvFetchPage(sid: number, offset: number) {
  const params = new URLSearchParams({
    token,
    session_id: String(sid),
    limit: String(HV_CHUNK_BYTES),
  });
  if (offset >= 0) params.set('offset', String(offset));
  const res = await fetch(`/api/session-log?${params.toString()}`);
  if (!res.ok) throw new Error(`session-log ${res.status}`);
  return res.json();
}

async function hvGoto(offset: number) {
  if (!hvState || hvLoading) return;
  hvLoading = true;
  try {
    const resp = await hvFetchPage(hvState.sid, offset);
    hvState.size = resp.size || 0;
    hvState.offset = resp.offset || 0;
    const term = ensureViewerTerm();
    if (!term) return;
    term.reset();
    const text = hvSanitize(hvDecodeChunk(resp.data_b64 || ''), hvState.offset);
    if (text) {
      term.write(text, () => { try { term.scrollToTop(); } catch (_) {} });
    }
    hvUpdateRangeLabel();
  } catch (err) {
    console.warn('[history-viewer] load failed', err);
    showToast(hvLabel('hv_load_failed', 'ログの取得に失敗しました'));
  } finally {
    hvLoading = false;
  }
}

export function openHistoryViewer(sid: number) {
  if (!sessions.has(sid)) return;
  const root = ensureViewer();
  if (!root) return;
  hvApplyLabels();
  root.hidden = false;
  hideHistoryHint();
  hvState = { sid, size: 0, offset: 0 };
  const term = ensureViewerTerm();
  if (term && hvFit) {
    requestAnimationFrame(() => { try { hvFit.fit(); } catch (_) {} });
  }
  // 「上端より前が見たい」導線なので先頭ページから開く
  hvGoto(0);
}

export function closeHistoryViewer() {
  if (hvRoot) hvRoot.hidden = true;
  hvState = null;
  if (hvTerm) { try { hvTerm.reset(); } catch (_) {} }
}

// セッション切替・タブ切替時に閉じる（別セッションのログを誤表示しない）
export function resetHistoryViewerForSessionChange() {
  closeHistoryViewer();
  hideHistoryHint();
}

// Escape で閉じる
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && hvRoot && !hvRoot.hidden) closeHistoryViewer();
});

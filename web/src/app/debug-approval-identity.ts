// 一時観測: 承認ポップアップの本文（経緯・質問文）と選択肢が別々の質問の中身に
// 入れ替わって表示される不具合の追跡用（docs/local/bugfix_approval-bar-stale-options-scroll-mismatch_2026-08-19.md）。
// 2 つの層から記録する。
//   [data]  showOptions() に渡された時点の options と、そこから計算された candidateKey / sourceEpoch
//   [draw]  実際の描画関数（single-tabs / batch-tabs / multi）の差分スキップ判定の直前
// 本文と選択肢を必ず同じ 1 件の中に並べて記録するので、ズレが「データの時点で既に起きている」のか
// 「データは正しいのに描画がスキップされて古いまま残っている」のかを画面上で切り分けられる。
// ゲート: URL クエリ ?approvaldebug=1 のときのみ動作する。既定は完全に no-op。
// 記録内容は identity のハッシュ短縮形と、前置き・質問文・選択肢ラベルの各先頭数十文字のみで、
// 自由入力欄の内容と送信テキストは保存しない。原因が確定したら instrumentation.json ごと撤去する。

let enabledCache: boolean | null = null;

function isEnabled(): boolean {
  if (enabledCache === null) {
    try {
      enabledCache = new URLSearchParams(location.search).get('approvaldebug') === '1';
    } catch (_) {
      enabledCache = false;
    }
  }
  return enabledCache;
}

interface DebugEntry {
  ts: string;
  layer: 'data' | 'draw';
  sessionId: number;
  head: string;
  preamble: string;
  question: string;
  opts: string;
}

const MAX_ENTRIES = 18;
const entries: DebugEntry[] = [];
let panelEl: HTMLDivElement | null = null;

function shortText(s: unknown, n: number): string {
  return String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
}

// 選択肢はラベル先頭 24 文字までを最大 3 件。どの質問の選択肢が出ているかを識別する目的で、
// 全文は要らない（自由入力欄の内容・送信テキストは元から触らない）。
function optionSummary(list: any): string {
  if (!Array.isArray(list) || list.length === 0) return '(none)';
  const parts = list.slice(0, 3).map((o: any) => `${o?.num ?? '?'}.${shortText(o?.label || o?.title, 24)}`);
  return parts.join(' / ') + (list.length > 3 ? ` …+${list.length - 3}` : '');
}

// batch の sections は「質問の配列」なので、選択肢は各セクションの中にある。
function flattenForSummary(options: any): any[] {
  if (!Array.isArray(options)) return [];
  const hasSections = options.some((o: any) => Array.isArray(o?.options));
  if (!hasSections) return options;
  return options.flatMap((sec: any) => (sec?.options || []).map((o: any) => ({
    num: `${sec?.num ?? '?'}-${o?.num ?? '?'}`,
    label: o?.label,
  })));
}

function push(entry: DebugEntry): void {
  entries.unshift(entry);
  while (entries.length > MAX_ENTRIES) entries.pop();
  render();
}

// 層1: showOptions() の識別子計算直後（データが届いた時点）。
export function noteApprovalIdentityForDebug(
  sessionId: number,
  identity: { candidateKey: string; sourceEpoch: number; shape: string },
  options: any,
  skipped: boolean,
): void {
  if (!isEnabled()) return;
  const arr: any = options;
  push({
    ts: new Date().toLocaleTimeString('ja-JP', { hour12: false }),
    layer: 'data',
    sessionId,
    head: `${skipped ? '[skip]' : '[show]'} key=${shortText(identity.candidateKey, 14)} epoch=${identity.sourceEpoch} shape=${shortText(identity.shape, 14)}`,
    preamble: shortText(arr && arr._preamble, 60),
    question: shortText((arr && arr._question) || (arr && arr[0] && arr[0]._question), 40),
    opts: optionSummary(flattenForSummary(options)),
  });
}

// 層2: 各描画関数の差分スキップ判定の直前。sigSkipped=true なら DOM は書き換わらない。
export function noteApprovalRenderForDebug(
  sessionId: number,
  mode: string,
  preamble: unknown,
  question: unknown,
  options: any,
  sigSkipped: boolean,
): void {
  if (!isEnabled()) return;
  push({
    ts: new Date().toLocaleTimeString('ja-JP', { hour12: false }),
    layer: 'draw',
    sessionId,
    head: `${sigSkipped ? '[sigskip]' : '[draw]'} mode=${shortText(mode, 14)}`,
    preamble: shortText(preamble, 60),
    question: shortText(question, 40),
    opts: optionSummary(flattenForSummary(options)),
  });
}

function ensurePanel(): HTMLDivElement {
  if (panelEl && panelEl.isConnected) return panelEl;
  const el = document.createElement('div');
  el.id = 'approval-identity-debug-panel';
  el.style.cssText = 'position:fixed;right:8px;bottom:8px;z-index:99999;width:460px;max-height:46vh;overflow-y:auto;background:rgba(13,17,23,0.94);color:#e6edf3;font:11px/1.4 "Cascadia Mono",Consolas,monospace;padding:8px;border:1px solid #555;border-radius:6px;white-space:pre-wrap;pointer-events:none;';
  document.body.appendChild(el);
  panelEl = el;
  return el;
}

function render(): void {
  const el = ensurePanel();
  el.textContent = entries
    .map((e) => `${e.ts} s=${e.sessionId} ${e.layer} ${e.head}\n  pre: ${e.preamble || '(none)'}\n  q  : ${e.question || '(none)'}\n  opt: ${e.opts}`)
    .join('\n\n');
}

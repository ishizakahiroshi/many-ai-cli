// 一時観測: 承認ポップアップの本文（経緯・質問文）と選択肢が別々の質問の中身に
// 入れ替わって表示される不具合、および「スクロールすると昔の承認ポップアップが再び出る」症状の
// 追跡用（docs/local/bugfix_approval-bar-stale-options-scroll-mismatch_2026-08-19.md）。
// 4 つの層から記録する。
//   approval.scroll  xterm の onScroll。ユーザー操作由来か TUI 再描画由来かを atBottom / manual で区別する
//   approval.buf     検出層。xterm バッファ末尾の再走査（scanBuffer）がライブ検出した選択肢を
//                    置き換えた／補完したときだけ記録する。マーカーを使わない provider（Codex 等）の経路
//   approval.data    showOptions() に渡された時点の options と candidateKey / sourceEpoch
//   approval.draw    実際の描画関数（single-tabs / batch-tabs / multi）の差分スキップ判定の直前
// 本文と選択肢を必ず同じ 1 件の中に並べて記録するので、ズレが「データの時点で既に起きている」のか
// 「データは正しいのに描画がスキップされて古いまま残っている」のかを画面上で切り分けられる。
//
// ゲートは 2 層。build 側（MAI_DEBUG=1 でなければこのファイルは成果物に入らない）と、
// runtime 側（URL クエリ ?approvaldebug=1 のときだけ sink を登録する）。sink が
// 登録されなければ記録点は完全な no-op で、観測用の配列コピーすら作られない。
//
// 記録内容は identity のハッシュ短縮形と、前置き・質問文・選択肢ラベルの各先頭数十文字のみで、
// 自由入力欄の内容と送信テキストは保存しない。原因が確定したら instrumentation.json ごと撤去する。

import { registerProbeSink, type ProbeFields } from './probe.js';

function isEnabled(): boolean {
  try {
    return new URLSearchParams(location.search).get('approvaldebug') === '1';
  } catch (_) {
    return false;
  }
}

interface DebugEntry {
  ts: string;
  layer: 'data' | 'draw' | 'buf' | 'scrl';
  sessionId: number;
  head: string;
  // 3 行の本体。層ごとに意味が違うのでラベルも一緒に持つ。
  // data / draw は前置き・質問文・選択肢、buf はライブ検出・バッファ検出・ゲート理由。
  rows: Array<[string, string]>;
}

const MAX_ENTRIES = 18;
const entries: DebugEntry[] = [];
let panelEl: HTMLDivElement | null = null;

function nowLabel(): string {
  return new Date().toLocaleTimeString('ja-JP', { hour12: false });
}

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

function num(v: unknown): number {
  return typeof v === 'number' ? v : Number(v) || 0;
}

// 記録点は「前・後・バッファ」の 3 つのスナップショットを渡してくるだけで、
// 何が起きたかの意味づけはここで決める（呼び元に観測用の状態機械を置かないため）。
function numsOf(list: any): string {
  return (Array.isArray(list) ? list : []).map((o: any) => o?.num).join(',');
}

function bufAction(live: any, after: any, buf: any): string {
  const l = numsOf(live);
  const a = numsOf(after);
  const b = numsOf(buf);
  if (l === a) return 'keep';
  if (a === b) return 'replace';
  return 'fill1';
}

// 層1: showOptions() の識別子計算直後（データが届いた時点）。
function noteData(f: ProbeFields): void {
  const identity = (f.identity || {}) as { candidateKey?: string; sourceEpoch?: number; shape?: string };
  const arr: any = f.options;
  push({
    ts: nowLabel(),
    layer: 'data',
    sessionId: num(f.sessionId),
    head: `${f.skipped ? '[skip]' : '[show]'} key=${shortText(identity.candidateKey, 14)} epoch=${identity.sourceEpoch} shape=${shortText(identity.shape, 14)}`,
    rows: [
      ['pre', shortText(arr && arr._preamble, 60)],
      ['q  ', shortText((arr && arr._question) || (arr && arr[0] && arr[0]._question), 40)],
      ['opt', optionSummary(flattenForSummary(f.options))],
    ],
  });
}

// 層2: 各描画関数の差分スキップ判定の直前。sigSkipped=true なら DOM は書き換わらない。
function noteDraw(f: ProbeFields): void {
  push({
    ts: nowLabel(),
    layer: 'draw',
    sessionId: num(f.sessionId),
    head: `${f.sigSkipped ? '[sigskip]' : '[draw]'} mode=${shortText(f.mode, 14)}`,
    rows: [
      ['pre', shortText(f.preamble, 60)],
      ['q  ', shortText(f.question, 40)],
      ['opt', optionSummary(flattenForSummary(f.options))],
    ],
  });
}

// 層3: 検出層。xterm バッファ末尾の再走査が結果を書き換えた瞬間だけ記録する。
// マーカー経路（handleHubApprovalMarker）はここを通らないので、記録が出れば
// 「マーカーを使わない provider の検出フォールバックが動いた」ことが確定する。
// 書き換えが無かった回（action='keep'）はここで捨てる。毎チャンク積むとパネルが埋まるため。
function noteBuf(f: ProbeFields): void {
  const action = bufAction(f.liveOptions, f.afterOptions, f.bufOptions);
  if (action === 'keep') return;
  const bufOpts: any = f.bufOptions;
  const bufHasCursor = Array.isArray(bufOpts) && bufOpts.some((o: any) => o?.isCurrent);
  push({
    ts: nowLabel(),
    layer: 'buf',
    sessionId: num(f.sessionId),
    head: `[${action}] site=${f.site} tail=${num(f.tailLines)} cur(live/buf)=${f.liveHasCursor ? 'Y' : 'n'}/${bufHasCursor ? 'Y' : 'n'}`,
    rows: [
      ['liv', optionSummary(flattenForSummary(f.liveOptions))],
      ['buf', optionSummary(flattenForSummary(f.bufOptions))],
      ['why', String(f.gate || '(none)')],
    ],
  });
}

// 層4: スクロール。連続発火をそのまま積むとパネルが埋まるので、
// 同じセッションの直近 1 件が scrl なら件数を数え上げて 1 行に畳む。
let lastScrollNoteAt = 0;

function noteScroll(f: ProbeFields): void {
  const sessionId = num(f.sessionId);
  const now = Date.now();
  const head = `[scroll] atBottom=${f.atBottom ? 'Y' : 'n'} manual=${f.manual ? 'Y' : 'n'}`;
  const pos = `viewportY=${num(f.viewportY)} baseY=${num(f.baseY)}`;
  const top = entries[0];
  if (top && top.layer === 'scrl' && top.sessionId === sessionId && now - lastScrollNoteAt < 1200) {
    const prev = Number(top.rows[0]?.[1]?.match(/^x(\d+)/)?.[1] || '1');
    top.rows[0] = ['cnt', `x${prev + 1}`];
    top.rows[1] = ['pos', pos];
    top.head = head;
    lastScrollNoteAt = now;
    render();
    return;
  }
  lastScrollNoteAt = now;
  push({
    ts: nowLabel(),
    layer: 'scrl',
    sessionId,
    head,
    rows: [
      ['cnt', 'x1'],
      ['pos', pos],
    ],
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
    .map((e) => {
      const body = e.rows.map(([label, value]) => `  ${label}: ${value || '(none)'}`).join('\n');
      return `${e.ts} s=${e.sessionId} ${e.layer} ${e.head}\n${body}`;
    })
    .join('\n\n');
}

// runtime ゲート。?approvaldebug=1 が無ければ sink を 1 つも登録しないので、
// 記録点は map 参照 1 回で戻る（渡したラムダは評価されない）。
if (isEnabled()) {
  registerProbeSink('approval.data', (_channel, fields) => noteData(fields));
  registerProbeSink('approval.draw', (_channel, fields) => noteDraw(fields));
  registerProbeSink('approval.buf', (_channel, fields) => noteBuf(fields));
  registerProbeSink('approval.scroll', (_channel, fields) => noteScroll(fields));
}

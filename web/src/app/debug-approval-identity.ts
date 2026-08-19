// 一時観測: 承認ポップアップの本文（経緯・質問文）と選択肢が別々の質問の中身に
// 入れ替わって表示される不具合の追跡用（docs/local/bugfix_approval-bar-stale-options-scroll-mismatch_2026-08-19.md）。
// showOptions() に渡された時点の options（_preamble / _question を含む）と、そこから
// 計算された candidateKey / sourceEpoch を、画面右下の固定パネルへ時系列で並べて表示する。
// ゲート: URL クエリ ?approvaldebug=1 のときのみ動作する。既定は完全に no-op。
// 記録内容は identity のハッシュ短縮形と前置き/質問文の先頭60文字のみで、選択肢の全文・
// ユーザー入力欄の内容は保存しない。原因が確定したら instrumentation.json ごと撤去する。

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
  sessionId: number;
  candidateKey: string;
  sourceEpoch: number;
  shape: string;
  skipped: boolean;
  preview: string;
}

const MAX_ENTRIES = 14;
const entries: DebugEntry[] = [];
let panelEl: HTMLDivElement | null = null;

function shortText(s: unknown, n: number): string {
  return String(s || '').replace(/\s+/g, ' ').trim().slice(0, n);
}

export function noteApprovalIdentityForDebug(
  sessionId: number,
  identity: { candidateKey: string; sourceEpoch: number; shape: string },
  options: any,
  skipped: boolean,
): void {
  if (!isEnabled()) return;
  const preambleOrQuestion = (options && (options._preamble || options._question)) || '';
  const firstLabel = Array.isArray(options) && options[0] ? (options[0].label || options[0].title || '') : '';
  entries.unshift({
    ts: new Date().toLocaleTimeString('ja-JP', { hour12: false }),
    sessionId,
    candidateKey: shortText(identity.candidateKey, 14),
    sourceEpoch: identity.sourceEpoch,
    shape: shortText(identity.shape, 14),
    skipped,
    preview: shortText(preambleOrQuestion || firstLabel, 70),
  });
  while (entries.length > MAX_ENTRIES) entries.pop();
  render();
}

function ensurePanel(): HTMLDivElement {
  if (panelEl && panelEl.isConnected) return panelEl;
  const el = document.createElement('div');
  el.id = 'approval-identity-debug-panel';
  el.style.cssText = 'position:fixed;right:8px;bottom:8px;z-index:99999;width:420px;max-height:40vh;overflow-y:auto;background:rgba(13,17,23,0.94);color:#e6edf3;font:11px/1.4 "Cascadia Mono",Consolas,monospace;padding:8px;border:1px solid #555;border-radius:6px;white-space:pre-wrap;pointer-events:none;';
  document.body.appendChild(el);
  panelEl = el;
  return el;
}

function render(): void {
  const el = ensurePanel();
  el.textContent = entries
    .map((e) => `${e.ts} s=${e.sessionId} ${e.skipped ? '[skip]' : '[show]'} key=${e.candidateKey} epoch=${e.sourceEpoch} shape=${e.shape}\n  ${e.preview}`)
    .join('\n\n');
}

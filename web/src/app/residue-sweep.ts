import { sendText } from '../app.js';
import { scanBuffer } from './terminal.js';
import { hasPendingDeferredEnter } from './deferred-enter.js';
import { activeSessionId, sessions } from './state.js';
import { shouldSkipClearPrefix } from './approval.js';

// 送信テキストの「残骸掃除」を事後検知で行うモジュール。
//
// 背景: かつては doSend / sendQuickCommand が全送信に \x15(Ctrl+U) を前置して
// 入力行を盲目的にクリアしていた（2026-06-09 導入。残骸への連結事故対策）。
// しかしこの方式は「相手プロンプトが Ctrl+U を行クリアと解釈する」仮定に依存し、
// shell（PowerShell でリテラル ^U 混入）→ codex（解釈せず \r 不達）→ claude /login の
// コード入力欄（リテラル混入で OAuth code が 1 文字汚れて 400）と、仮定が崩れるたびに
// 例外が増えた。前置は全廃し、本モジュールが置き換える。
//
// 方針: 端末ペインは表示専用（disableStdin）で、内側 CLI の入力行に文字を入れられるのは
// Hub 自身だけ。だから「送ったテキストが確定されず入力行に張り付いたままか」を送信後に
// 観測し、張り付きを実際に見たときだけ \x15 を 1 回撃って掃除する。
//
// 安全性の要: \x15 を送るのは「送信テキストの平文が画面下部（入力行領域）に残っている」のを
// 確認できたときだけ。平文をエコーするのは通常チャット入力欄＝ Ctrl+U が効く文脈であり、
// /login のようなマスク入力（エコーは *）は原理的にマッチしないため誤爆しない。

const IDLE_SETTLE_MS = 1500; // この時間 新たな出力が来なければ判定してよいとみなす
const MIN_WAIT_MS = 2500;    // 送信直後のエコー・畳み込み反映を待つ最低時間
const RETRY_MS = 5000;       // 非アクティブ等で判定できない時の再試行間隔
const MAX_RETRIES = 60;      // 約 5 分で打ち切り（残骸は手動の入力クリアでも掃除できる）
const SCAN_LINES = 12;       // バッファ末尾から入力行領域とみなす行数
const MIN_FRAGMENT_LEN = 5;  // これ未満の断片は誤マッチしやすいので掃除対象にしない

type Pending = {
  startedAt: number;
  idleTimer: ReturnType<typeof setTimeout> | null;
  retries: number;
  fragments: string[];
};

const pending = new Map<number, Pending>();

// 送信テキストから「入力行に残っていたら張り付きとみなす」断片を作る。
// 折り返し・入力枠の装飾を跨がないよう先頭 24 文字に切り詰める。
function buildFragments(rawText: string, isMultiLine: boolean): string[] {
  const fragments: string[] = [];
  const firstLine = (rawText.split('\n').find((l) => l.trim() !== '') || '').trim();
  const head = firstLine.slice(0, 24);
  if (head.length >= MIN_FRAGMENT_LEN) fragments.push(head);
  // 複数行ペーストは内側 CLI がプレースホルダへ畳み込むため平文では残らない。
  // 張り付き時に画面へ残るのはプレースホルダの方（Claude Code: "[Pasted text #N +M lines]"）。
  if (isMultiLine) fragments.push('[Pasted text');
  return fragments;
}

function fire(id: number) {
  const p = pending.get(id);
  if (!p) return;
  if (!sessions.has(id)) { pending.delete(id); return; }
  // 確定 \r やペースト本体の送出が未了なら判定しない（送信途中の内容を消さないため）
  if (hasPendingDeferredEnter(id)) { retryLater(id); return; }
  // 非アクティブセッションの xterm バッファはライブ書き込みが止まっていて古い。
  // アクティブになる（= flushPending で最新化される）まで判定を先送りする。
  if (id !== activeSessionId) { retryLater(id); return; }
  pending.delete(id);
  const tail = scanBuffer(id, SCAN_LINES);
  const stuck = tail.some((line) => p.fragments.some((f) => line.includes(f)));
  if (stuck) sendText(id, '\x15');
}

function retryLater(id: number) {
  const p = pending.get(id);
  if (!p) return;
  p.retries++;
  if (p.retries > MAX_RETRIES) { pending.delete(id); return; }
  if (p.idleTimer) clearTimeout(p.idleTimer);
  p.idleTimer = setTimeout(() => fire(id), RETRY_MS);
}

function armIdle(id: number) {
  const p = pending.get(id);
  if (!p) return;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  const elapsed = Date.now() - p.startedAt;
  const wait = Math.max(IDLE_SETTLE_MS, MIN_WAIT_MS - elapsed);
  p.idleTimer = setTimeout(() => fire(id), wait);
}

// doSend / sendQuickCommand から送信ごとに呼ぶ。既存予約は破棄して張り直す
// （新しい送信があった時点で、前の送信の張り付き判定は新送信の観測に置き換わる）。
export function scheduleResidueSweep(id: number, rawText: string, isMultiLine: boolean = false) {
  cancelResidueSweep(id);
  if (shouldSkipClearPrefix(sessions.get(id)?.provider || '')) return;
  const fragments = buildFragments(rawText, isMultiLine);
  if (fragments.length === 0) return;
  pending.set(id, { startedAt: Date.now(), idleTimer: null, retries: 0, fragments });
  armIdle(id);
}

// pty_data 受信ごとに呼ぶ。出力が続く間（= 内側 CLI が取り込み・応答中）は判定を遅らせる。
export function notifyResidueSweepOutput(id: number) {
  if (pending.has(id)) armIdle(id);
}

export function cancelResidueSweep(id: number) {
  const p = pending.get(id);
  if (!p) return;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  pending.delete(id);
}

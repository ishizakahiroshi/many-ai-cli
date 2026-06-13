import { sendText } from '../app.js';

// 複数行ペースト送信の確定 \r を「いつ送るか」を出力駆動で決めるモジュール。
//
// 背景: ブラケットペーストで本文を送った直後に確定 \r を送ると、内側 CLI（特に Codex /
// Windows ConPTY）が paste の取り込み・再描画の最中で \r を取りこぼし、入力欄にテキストが
// 張り付いたまま実行されない（実測: 約900字の複数行ペーストで再発）。固定遅延（120ms 等）は
// ペースト長・環境速度に依存して当たり外れがあり、いたちごっこになる。
//
// 方針: 確定 \r を「PTY 出力が一定時間 静止した（= 取り込み・再描画が落ち着いた）」のを
// 待ってから 1 回だけ送る。各 pty_data チャンク受信で待機をリセットし、出力が止まったら送出する。
// 出力駆動なのでペースト長に依存せず自己調整し、\r は必ず 1 回しか送らないため二重送信
// （確定直後の余分な \r による後続プロンプト誤承認）が原理的に起きない。
//
// この「出力静止後に 1 回だけ撃つ」土台は scheduleAfterOutputSettle として一般化してあり、
// 画像 inject（@path）に複数行ペーストが続くケースで「画像取り込みが落ち着いてからペースト本体を
// 送る」前段にも使う。確定 \r 区間（scheduleDeferredEnter）の挙動は従来と不変。

const IDLE_SETTLE_MS = 120; // この時間 新たな出力が来なければ「描画が落ち着いた」とみなす
const MIN_WAIT_MS = 120;    // 送出直後はまだ再描画が始まっていないため、最低この時間は待つ
// 出力が止まらなくても、この時間で必ず action を実行する（保険）。
// これは「出力が永久に止まらない病的ケース」専用の安全網であって、通常の取り込み・畳み込み
// 時間と競合させてはいけない。短すぎると、巨大ペースト（実測 422 行）の畳み込みが終わる前に
// \r を強制発火し、ビジー中の内側 CLI に \r が吸収されて入力欄に張り付いたまま永久フリーズする
// （単発設計なので再送されない / 「Pasting…」固着）。畳み込み中は pty_data が出続けるので
// 本来は idle 側が畳み込み完了後に正しく発火する。保険はそれを横取りしない大きさにする。
const MAX_WAIT_MS = 30000;
// 画像 inject（@path）取り込みの静止待ち専用の保険。\r 確定と違い、ここで撃つのはペースト本体
// 送出なので、出力が止まらなくてもペースト送出を不必要に遅らせないよう短めに切る。発火しても
// ペースト本体は送られ、その後の確定 \r は通常の scheduleDeferredEnter が引き継ぐ。
const INJECT_SETTLE_MAX_WAIT_MS = 4000;

type Pending = {
  startedAt: number;
  idleTimer: ReturnType<typeof setTimeout> | null;
  maxTimer: ReturnType<typeof setTimeout> | null;
  fired: boolean;
  action: () => void;
};

const pending = new Map<number, Pending>();

function fire(id: number) {
  const p = pending.get(id);
  if (!p || p.fired) return;
  p.fired = true;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  if (p.maxTimer) clearTimeout(p.maxTimer);
  pending.delete(id);
  try { p.action(); } catch (_) {}
}

function armIdle(id: number) {
  const p = pending.get(id);
  if (!p || p.fired) return;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  const elapsed = Date.now() - p.startedAt;
  // 最低 MIN_WAIT は確保しつつ、以降は出力静止 IDLE_SETTLE ごとに送出判定する
  const wait = Math.max(IDLE_SETTLE_MS, MIN_WAIT_MS - elapsed);
  p.idleTimer = setTimeout(() => fire(id), wait);
}

// 出力が一定時間静止してから action を 1 回だけ実行する予約。\r 確定・ペースト本体送出など
// 「内側 CLI の取り込み・再描画が落ち着いてから撃ちたい」操作の共通土台。
function schedule(id: number, action: () => void, maxWaitMs: number) {
  cancelDeferredEnter(id);
  const p: Pending = { startedAt: Date.now(), idleTimer: null, maxTimer: null, fired: false, action };
  pending.set(id, p);
  p.maxTimer = setTimeout(() => fire(id), maxWaitMs);
  armIdle(id);
}

// doSend の deferEnter 分岐から呼ぶ。確定 \r を出力静止後に 1 回だけ送る予約を張る。
export function scheduleDeferredEnter(id: number) {
  schedule(id, () => { try { sendText(id, '\r'); } catch (_) {} }, MAX_WAIT_MS);
}

// 画像 inject（@path）に複数行ペーストが続くケースで、画像取り込みが落ち着いてから
// ペースト本体＋確定 \r を撃つために使う。出力静止後に action を 1 回だけ実行する。
export function scheduleAfterOutputSettle(id: number, action: () => void) {
  schedule(id, action, INJECT_SETTLE_MAX_WAIT_MS);
}

// pty_data 受信ごとに呼ぶ。出力が続く間は \r を遅らせ、静止したら送る。
export function notifyDeferredEnterOutput(id: number) {
  const p = pending.get(id);
  if (!p || p.fired) return;
  armIdle(id);
}

export function cancelDeferredEnter(id: number) {
  const p = pending.get(id);
  if (!p) return;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  if (p.maxTimer) clearTimeout(p.maxTimer);
  pending.delete(id);
}

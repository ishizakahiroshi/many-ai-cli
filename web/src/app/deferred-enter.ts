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

const IDLE_SETTLE_MS = 120; // この時間 新たな出力が来なければ「描画が落ち着いた」とみなす
const MIN_WAIT_MS = 120;    // 送出直後はまだ再描画が始まっていないため、最低この時間は待つ
// 出力が止まらなくても、この時間で必ず \r を送る（保険）。
// これは「出力が永久に止まらない病的ケース」専用の安全網であって、通常の取り込み・畳み込み
// 時間と競合させてはいけない。短すぎると、巨大ペースト（実測 422 行）の畳み込みが終わる前に
// \r を強制発火し、ビジー中の内側 CLI に \r が吸収されて入力欄に張り付いたまま永久フリーズする
// （単発設計なので再送されない / 「Pasting…」固着）。畳み込み中は pty_data が出続けるので
// 本来は idle 側が畳み込み完了後に正しく発火する。保険はそれを横取りしない大きさにする。
const MAX_WAIT_MS = 30000;

type Pending = {
  startedAt: number;
  idleTimer: ReturnType<typeof setTimeout> | null;
  maxTimer: ReturnType<typeof setTimeout> | null;
  fired: boolean;
};

const pending = new Map<number, Pending>();

function fire(id: number) {
  const p = pending.get(id);
  if (!p || p.fired) return;
  p.fired = true;
  if (p.idleTimer) clearTimeout(p.idleTimer);
  if (p.maxTimer) clearTimeout(p.maxTimer);
  pending.delete(id);
  try { sendText(id, '\r'); } catch (_) {}
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

// doSend の deferEnter 分岐から呼ぶ。確定 \r を出力静止後に 1 回だけ送る予約を張る。
export function scheduleDeferredEnter(id: number) {
  cancelDeferredEnter(id);
  const p: Pending = { startedAt: Date.now(), idleTimer: null, maxTimer: null, fired: false };
  pending.set(id, p);
  p.maxTimer = setTimeout(() => fire(id), MAX_WAIT_MS);
  armIdle(id);
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

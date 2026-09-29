// window-channel.ts — Hub の本体の窓と別窓（/?view=detached-tab）の連絡路。
// plan: docs/local/plan_detached-tab-windows.md C3
//
// 同じ origin の窓どうしで BroadcastChannel を 1 本使う。循環 import の輪に入るモジュール
// （session-list.ts / memo-panel.ts 等）から使われるので、ここは何も import しない。
// 各窓はそれぞれ自分の /ws を持っており、セッションやメモの中身はこの経路で運ばない。
// 運ぶのは「どれを選んでいるか」と「本体で開いてほしい」という依頼だけ。

export type WindowMessage =
  // 本体 → 別窓: 本体で選んでいるセッション
  | { type: 'active-session'; sessionId: number | null }
  // 別窓 → 本体: 開いた直後の挨拶。本体は active-session を返す
  | { type: 'hello' }
  // 別窓 → 本体: 本体でこのセッションを開いてほしい
  | { type: 'open-session'; sessionId: number; requestId: string }
  // 別窓 → 本体: 本体で起動画面を開いてほしい
  | { type: 'open-spawn'; cwd: string; prompt?: string; provider?: string; requestId: string }
  // 本体 → 別窓: 依頼を受け取った
  | { type: 'ack'; requestId: string }
  // 全窓: 作業メモが変わったので取り直してほしい
  | { type: 'memos-changed' };

const CHANNEL_NAME = 'many-ai-cli-windows';
const ACK_TIMEOUT_MS = 1000;

let channel: BroadcastChannel | null | undefined;
const listeners = new Set<(msg: WindowMessage) => void>();
const pendingAcks = new Map<string, (ok: boolean) => void>();

function getChannel(): BroadcastChannel | null {
  if (channel !== undefined) return channel;
  if (typeof BroadcastChannel === 'undefined') {
    channel = null;
    return channel;
  }
  channel = new BroadcastChannel(CHANNEL_NAME);
  channel.onmessage = (event: MessageEvent) => {
    const msg = event.data as WindowMessage;
    if (!msg || typeof msg !== 'object' || typeof (msg as { type?: unknown }).type !== 'string') return;
    if (msg.type === 'ack') {
      const done = pendingAcks.get(msg.requestId);
      if (done) { pendingAcks.delete(msg.requestId); done(true); }
      return;
    }
    listeners.forEach((fn) => {
      try { fn(msg); } catch (err) { console.warn('[window-channel]', err); }
    });
  };
  return channel;
}

export function postWindowMessage(msg: WindowMessage): void {
  try { getChannel()?.postMessage(msg); } catch (_) { /* 閉じかけの窓では失敗してよい */ }
}

export function onWindowMessage(fn: (msg: WindowMessage) => void): () => void {
  getChannel();
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}

type RequestMessage =
  | { type: 'open-session'; sessionId: number }
  | { type: 'open-spawn'; cwd: string; prompt?: string; provider?: string };

/**
 * 本体の窓へ依頼する。本体が受け取れば true、1 秒待っても返事が無ければ false
 * （本体の窓が開いていない）。
 */
export function requestMainWindow(msg: RequestMessage): Promise<boolean> {
  if (!getChannel()) return Promise.resolve(false);
  const requestId = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      pendingAcks.delete(requestId);
      resolve(false);
    }, ACK_TIMEOUT_MS);
    pendingAcks.set(requestId, (ok) => { clearTimeout(timer); resolve(ok); });
    postWindowMessage({ ...msg, requestId } as WindowMessage);
  });
}

export function ackWindowRequest(requestId: string): void {
  postWindowMessage({ type: 'ack', requestId });
}

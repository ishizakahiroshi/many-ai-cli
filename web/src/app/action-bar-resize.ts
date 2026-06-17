// 質問エリア（#action-bar）の縦方向リサイズ。
// 上端 6px 帯を pointerdown で掴むと max-height を伸縮できる。
// 上端帯のダブルクリックでユーザー指定高さを解除（CSS のデフォルトに戻す）。
//
// ポップアップを伸縮したぶんは、中の質問欄（.action-preamble-body＝経緯ボックス）へ
// 同量だけ注いで一緒に伸縮させる（広げた分の空白を質問欄が埋め、縮めた分は戻す）。
// 質問欄が無い承認（前置きなし）のときはバーの伸縮のみ行う。

import { followActionBarResize } from './terminal';

const HANDLE_ZONE_PX = 8;
const MIN_PX = 80;
const MIN_PREAMBLE_PX = 44;

export function initActionBarResize(): void {
  const bar = document.getElementById('action-bar');
  if (!bar) return;

  // バー伸縮中はアクティブターミナルを同フレームで再フィット＆最下部追従させる。
  // pointermove は高頻度なので rAF で 1 フレーム 1 回に間引く。
  let followRafPending = false;
  const followTerminal = (): void => {
    if (followRafPending) return;
    followRafPending = true;
    requestAnimationFrame(() => {
      followRafPending = false;
      followActionBarResize();
    });
  };

  let dragging = false;
  let startY = 0;
  let startHeight = 0;
  let pointerId = -1;
  let preamble: HTMLElement | null = null;
  let startPreambleHeight = 0;

  const isOnHandle = (e: PointerEvent): boolean => {
    if (e.target !== bar) return false;
    const rect = bar.getBoundingClientRect();
    return e.clientY - rect.top <= HANDLE_ZONE_PX;
  };

  bar.addEventListener('pointerdown', (e) => {
    if (!isOnHandle(e)) return;
    dragging = true;
    pointerId = e.pointerId;
    startY = e.clientY;
    startHeight = bar.getBoundingClientRect().height;
    // ドラッグ開始時の質問欄の実描画高さを基準に、バー増減分を加算していく。
    preamble = bar.querySelector<HTMLElement>('.action-preamble-body');
    startPreambleHeight = preamble ? preamble.getBoundingClientRect().height : 0;
    bar.classList.add('resizing');
    try { bar.setPointerCapture(e.pointerId); } catch {}
    e.preventDefault();
  });

  bar.addEventListener('pointermove', (e) => {
    if (!dragging || e.pointerId !== pointerId) return;
    const dy = startY - e.clientY;
    const next = Math.max(MIN_PX, Math.min(window.innerHeight - 120, startHeight + dy));
    bar.style.maxHeight = next + 'px';
    bar.style.height = next + 'px';
    if (preamble) {
      // バーが実際に伸縮した量（クランプ後）を質問欄へそのまま注ぐ。
      const barDelta = next - startHeight;
      const nextPreamble = Math.max(MIN_PREAMBLE_PX, startPreambleHeight + barDelta);
      preamble.style.maxHeight = nextPreamble + 'px';
    }
    followTerminal();
  });

  const end = (e: PointerEvent) => {
    if (!dragging || e.pointerId !== pointerId) return;
    dragging = false;
    pointerId = -1;
    preamble = null;
    bar.classList.remove('resizing');
    try { bar.releasePointerCapture(e.pointerId); } catch {}
    followTerminal();
  };
  bar.addEventListener('pointerup', end);
  bar.addEventListener('pointercancel', end);

  bar.addEventListener('dblclick', (e) => {
    if (e.target !== bar) return;
    const rect = bar.getBoundingClientRect();
    if (e.clientY - rect.top > HANDLE_ZONE_PX) return;
    bar.style.maxHeight = '';
    bar.style.height = '';
    // 質問欄も CSS デフォルト（max-height: 5.5em / 展開時 22em）へ戻す。
    const body = bar.querySelector<HTMLElement>('.action-preamble-body');
    if (body) body.style.maxHeight = '';
    followTerminal();
  });
}

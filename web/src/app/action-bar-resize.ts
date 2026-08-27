// 質問エリア（#action-bar）のリサイズ。
//
// 下ドック（既定）:
//   上端 8px 帯を pointerdown で掴むと max-height を伸縮できる。
//   上端帯のダブルクリックでユーザー指定高さを解除（CSS のデフォルトに戻す）。
// 右/左カラム（body.approval-dock-right / -left。状態は ui-side.ts の uiSideApproval）:
//   内側の縦辺 8px 帯で幅 --approval-dock-w を伸縮する。ダブルクリックで既定幅へ戻す。
//
// 下ドック時は、ポップアップを伸縮したぶんを中の質問欄（.action-preamble-body＝経緯ボックス）へ
// 同量だけ注いで一緒に伸縮させる（広げた分の空白を質問欄が埋め、縮めた分は戻す）。
// 質問欄が無い承認（前置きなし）のときはバーの伸縮のみ行う。カラム時は高さが全固定なので
// 質問欄への注ぎは行わない。
//
// 下ドック ⇄ カラムの切替時に、前の向きでドラッグした inline サイズ（width/height/max-height）
// を消す。inline width が残ったまま下ドックへ戻ると横幅が狭いまま貼り付き、逆も同様のため。

import { followActionBarResize } from './terminal.js';

const HANDLE_ZONE_PX = 8;
const MIN_PX = 80;
const MIN_W_PX = 240;
const MIN_PREAMBLE_PX = 44;

type Axis = 'y' | 'x';

function approvalColumn(): '' | 'right' | 'left' {
  if (document.body.classList.contains('approval-dock-right')) return 'right';
  if (document.body.classList.contains('approval-dock-left')) return 'left';
  return '';
}

export function initActionBarResize(): void {
  const bar = document.getElementById('action-bar');
  if (!bar) return;

  const clearInlineSize = (): void => {
    bar.style.maxHeight = '';
    bar.style.height = '';
    bar.style.width = '';
    // 質問欄へ注いだ高さも CSS デフォルト（max-height: 5.5em / 展開時 22em）へ戻す。
    const body = bar.querySelector<HTMLElement>('.action-preamble-body');
    if (body) body.style.maxHeight = '';
  };

  // 向きが変わったら inline サイズを掃除する（下ドック ⇄ カラムで相互汚染しないように）。
  let lastMode = approvalColumn();
  try {
    if (typeof MutationObserver === 'function') {
      new MutationObserver(() => {
        const mode = approvalColumn();
        if (mode !== lastMode) {
          lastMode = mode;
          clearInlineSize();
          followActionBarResize();
        }
      }).observe(document.body, { attributes: true, attributeFilter: ['class'] });
    }
  } catch (_) { /* 監視できない環境では掃除のみ skip */ }

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
  let axis: Axis = 'y';
  let dragSign = 1;
  let startPos = 0;
  let startSize = 0;
  let pointerId = -1;
  let preamble: HTMLElement | null = null;
  let startPreambleHeight = 0;

  // カラム時は内側の縦辺（右カラムなら左辺・左カラムなら右辺）が当たり判定。
  const isOnHandle = (e: PointerEvent): boolean => {
    if (e.target !== bar) return false;
    const rect = bar.getBoundingClientRect();
    const col = approvalColumn();
    if (col === 'right') return e.clientX - rect.left <= HANDLE_ZONE_PX;
    if (col === 'left') return rect.right - e.clientX <= HANDLE_ZONE_PX;
    return e.clientY - rect.top <= HANDLE_ZONE_PX;
  };

  bar.addEventListener('pointerdown', (e) => {
    if (!isOnHandle(e)) return;
    dragging = true;
    pointerId = e.pointerId;
    const rect = bar.getBoundingClientRect();
    const col = approvalColumn();
    if (col) {
      axis = 'x';
      // 右カラムは内側辺（左辺）から離れる方向＝左へ動かすと広がる。
      // 左カラムは内側辺（右辺）から離れる方向＝右へ動かすと広がる。
      dragSign = col === 'right' ? -1 : 1;
      startPos = e.clientX;
      startSize = rect.width;
    } else {
      axis = 'y';
      dragSign = -1; // 下端固定で上へ動かすと広がる
      startPos = e.clientY;
      startSize = rect.height;
      // ドラッグ開始時の質問欄の実描画高さを基準に、バー増減分を加算していく。
      preamble = bar.querySelector<HTMLElement>('.action-preamble-body');
      startPreambleHeight = preamble ? preamble.getBoundingClientRect().height : 0;
    }
    bar.classList.add('resizing');
    try { bar.setPointerCapture(e.pointerId); } catch {}
    e.preventDefault();
  });

  bar.addEventListener('pointermove', (e) => {
    if (!dragging || e.pointerId !== pointerId) return;
    const delta = dragSign * (e[axis] - startPos);
    if (axis === 'y') {
      const next = Math.max(MIN_PX, Math.min(window.innerHeight - 120, startSize + delta));
      bar.style.maxHeight = next + 'px';
      bar.style.height = next + 'px';
      if (preamble) {
        // バーが実際に伸縮した量（クランプ後）を質問欄へそのまま注ぐ。
        const barDelta = next - startSize;
        const nextPreamble = Math.max(MIN_PREAMBLE_PX, startPreambleHeight + barDelta);
        preamble.style.maxHeight = nextPreamble + 'px';
      }
    } else {
      const next = Math.max(MIN_W_PX, Math.min(window.innerWidth - 320, startSize + delta));
      bar.style.width = next + 'px';
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
    const col = approvalColumn();
    if (col === 'right' && e.clientX - rect.left > HANDLE_ZONE_PX) return;
    if (col === 'left' && rect.right - e.clientX > HANDLE_ZONE_PX) return;
    if (!col && e.clientY - rect.top > HANDLE_ZONE_PX) return;
    clearInlineSize();
    followTerminal();
  });
}

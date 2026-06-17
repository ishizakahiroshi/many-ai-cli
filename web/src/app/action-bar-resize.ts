// 質問エリア（#action-bar）の縦方向リサイズ。
// 上端 6px 帯を pointerdown で掴むと max-height を伸縮できる。
// 上端帯のダブルクリックでユーザー指定高さを解除（CSS のデフォルトに戻す）。

const HANDLE_ZONE_PX = 8;
const MIN_PX = 80;

export function initActionBarResize(): void {
  const bar = document.getElementById('action-bar');
  if (!bar) return;

  let dragging = false;
  let startY = 0;
  let startHeight = 0;
  let pointerId = -1;

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
  });

  const end = (e: PointerEvent) => {
    if (!dragging || e.pointerId !== pointerId) return;
    dragging = false;
    pointerId = -1;
    bar.classList.remove('resizing');
    try { bar.releasePointerCapture(e.pointerId); } catch {}
  };
  bar.addEventListener('pointerup', end);
  bar.addEventListener('pointercancel', end);

  bar.addEventListener('dblclick', (e) => {
    if (e.target !== bar) return;
    const rect = bar.getBoundingClientRect();
    if (e.clientY - rect.top > HANDLE_ZONE_PX) return;
    bar.style.maxHeight = '';
    bar.style.height = '';
  });
}

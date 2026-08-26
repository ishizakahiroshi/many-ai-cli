// セッションカードの列（#session-list）を画面の左右どちらに置くかを切り替える。
//
// 既定は左。右へ寄せると、タブバー右端の各種 ✕ / ↻ とカード列が同じ側に並ぶので、
// 「カードで切り替える → パネルを閉じる」の往復でマウスの移動距離が短くなる。
//
// 設計メモ:
//  - DOM は動かさず flex の order だけを入れ替える（#app は display:flex）。
//    DOM を並べ替えると detached-grid の hideIds や モバイルドロワーの前提が崩れる。
//  - モバイル幅ではカード列が position:fixed のドロワーになり order が効かないため、
//    CSS 側でデスクトップ幅にだけ適用している。
//  - 保存先は localStorage（ブラウザ単位）。タブの並び順（tab-bar-order.ts）と同じく
//    端末ごとに好みが違う設定なので、サーバー設定には持たせない。

const STORAGE_KEY = 'sessionListSide';

export type SidebarSide = 'left' | 'right';

export function getSidebarSide(): SidebarSide {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'right' ? 'right' : 'left';
  } catch (_) {
    return 'left';
  }
}

function apply(side: SidebarSide): void {
  document.body.classList.toggle('sidebar-right', side === 'right');
}

export function setSidebarSide(side: SidebarSide): void {
  try { localStorage.setItem(STORAGE_KEY, side); } catch (_) { /* private mode 等は無視 */ }
  apply(side);
  // 端末の実幅は変わらないので pty_resize は不要。位置だけが入れ替わる。
}

export function initSidebarSide(): void {
  apply(getSidebarSide());

  const sel = document.getElementById('sidebar-side-select') as HTMLSelectElement | null;
  if (!sel) return;
  sel.value = getSidebarSide();
  sel.addEventListener('change', () => {
    setSidebarSide(sel.value === 'right' ? 'right' : 'left');
  });
}

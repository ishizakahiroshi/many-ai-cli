// サイドバーのタブ（箱の切り替え）を、上の横一列にするか、左の縦レールにするかを持つ。
//
// 値は 'horizontal'（既定・従来どおり）と 'vertical'。設定 → 配置 の選択欄で選ぶ。
// 保存先は localStorage（ブラウザ単位）。ui-side.ts の左右設定と同じ扱いで、サーバー設定には持たせない。
//
// 設計メモ:
//  - DOM は動かさない。#session-list に tabs-vertical クラスを付け、並びは CSS（session-list-tabs.css）で
//    横一列から縦レールへ組み替える。DOM を並べ替えるとモバイルドロワーや detached-grid の前提が崩れる。
//  - 縦レールが効くのは「マウスで使う PC の幅」だけ（CSS の @media と同じ条件。VERTICAL_TABS_QUERY）。
//    スマホ幅とタッチ操作ではドロワーの横並びのまま。保存は変えず、見た目だけ横へ戻る。
//  - 並び・ドラッグ・矢印キーの向きは JS も同じ条件で読む（isSessionListTabsVertical）。
//    CSS と JS の条件を 1 か所（この定数）に揃えて、片方だけ縦になる食い違いを防ぐ。

export type SessionListTabLayout = 'horizontal' | 'vertical';

const STORAGE_KEY = 'sessionListTabsLayout';

/** session-list-tabs.css の縦レールの @media と同じ条件。変えるときは両方を直す。 */
export const VERTICAL_TABS_QUERY = '(min-width: 721px) and (not (pointer: coarse))';

export function getSessionListTabLayout(): SessionListTabLayout {
  try { return localStorage.getItem(STORAGE_KEY) === 'vertical' ? 'vertical' : 'horizontal'; }
  catch (_) { return 'horizontal'; }
}

function verticalAvailable(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(VERTICAL_TABS_QUERY).matches;
}

/** いま画面で縦レールとして描かれているか（設定が縦 かつ PC の幅・マウス操作）。 */
export function isSessionListTabsVertical(): boolean {
  return getSessionListTabLayout() === 'vertical' && verticalAvailable();
}

function applyLayout(): void {
  const vertical = getSessionListTabLayout() === 'vertical';
  document.getElementById('session-list')?.classList.toggle('tabs-vertical', vertical);
  document.getElementById('session-list-tabs')
    ?.setAttribute('aria-orientation', isSessionListTabsVertical() ? 'vertical' : 'horizontal');
  const select = document.getElementById('session-list-tab-layout-select') as HTMLSelectElement | null;
  if (select && select.value !== getSessionListTabLayout()) select.value = getSessionListTabLayout();
}

export function setSessionListTabLayout(layout: SessionListTabLayout): void {
  try {
    if (layout === 'vertical') localStorage.setItem(STORAGE_KEY, 'vertical');
    else localStorage.removeItem(STORAGE_KEY);
  } catch (_) { /* private mode 等は無視（この画面の間だけ効く） */ }
  applyLayout();
}

export function initSessionListTabLayout(): void {
  applyLayout();
  const select = document.getElementById('session-list-tab-layout-select') as HTMLSelectElement | null;
  select?.addEventListener('change', () => {
    setSessionListTabLayout(select.value === 'vertical' ? 'vertical' : 'horizontal');
  });
  // 窓の幅や入力方式が変わると、縦レールが効く・効かないが入れ替わる。aria-orientation を追従させる。
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(VERTICAL_TABS_QUERY).addEventListener('change', applyLayout);
  }
}

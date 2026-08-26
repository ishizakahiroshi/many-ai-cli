// UI の操作系を画面のどちら側へ寄せるかを 1 つの状態で持つ。
//
// 対象は 3 つ:
//   - セッションカードの列（#session-list）
//   - 入力欄のツール群（#input-wrap 内のボタン列。⇄ = #tools-flip-btn が従来から担当）
//   - 各パネルの閉じるボタン（Review / Git / 設定 / 各モーダルの ✕）
//
// 目的は「カードで切り替える → パネルを閉じる」の往復でマウスの移動距離を縮めること。
// 3 つがばらばらの側にあると往復が増えるので、状態は 1 つにして ⇄ ボタンと設定画面の
// 両方から同じ値を書き換える。
//
// 値は 3 つある:
//   ''      … 既定。従来どおり（カード列 左 / 入力欄ツールは旧キー / ✕ 右）。
//             既存ユーザーの画面を更新だけで動かさないために残している。
//   'left'  … カード列 左 / 入力欄ツール 左 / ✕ 左
//   'right' … カード列 右 / 入力欄ツール 右 / ✕ 右
//
// 設計メモ:
//  - DOM は動かさず flex の order だけを入れ替える（DOM を並べ替えると detached-grid の
//    要素隠しやモバイルドロワーの前提が崩れる）。入力欄ツールだけは従来から DOM を
//    並べ替えているので、そちらは app.ts の applyToolsPosition に委ねる。
//  - モバイル幅ではカード列が position:fixed のドロワーになり order が効かないため、
//    カード列の入れ替えは CSS 側でデスクトップ幅にだけ効かせている。
//  - 保存先は localStorage（ブラウザ単位）。端末ごとに好みが違う設定なので
//    サーバー設定には持たせない（タブの並び順と同じ扱い）。

import { STORAGE_TOOLS_LEFT_KEY } from './user-prefs.js';

export type UiSide = '' | 'left' | 'right';

const STORAGE_KEY = 'uiSide';

type Listener = (side: UiSide, toolsLeft: boolean) => void;
const listeners: Listener[] = [];

export function getUiSide(): UiSide {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === 'left' || v === 'right' ? v : '';
  } catch (_) {
    return '';
  }
}

/** 入力欄ツールを左に置くか。既定（''）のときだけ従来キーを見る。 */
export function toolsOnLeft(): boolean {
  const side = getUiSide();
  if (side) return side === 'left';
  try { return localStorage.getItem(STORAGE_TOOLS_LEFT_KEY) === '1'; } catch (_) { return false; }
}

function applyBodyClasses(side: UiSide): void {
  document.body.classList.toggle('sidebar-right', side === 'right');
  document.body.classList.toggle('close-left', side === 'left');
}

export function setUiSide(side: UiSide): void {
  try {
    if (side) localStorage.setItem(STORAGE_KEY, side);
    else localStorage.removeItem(STORAGE_KEY);
    // 旧キーも同じ値に保つ（読む側が増えても状態が割れないように）
    if (side) localStorage.setItem(STORAGE_TOOLS_LEFT_KEY, side === 'left' ? '1' : '0');
  } catch (_) { /* private mode 等は無視 */ }
  applyBodyClasses(side);
  const sel = document.getElementById('ui-side-select') as HTMLSelectElement | null;
  if (sel && sel.value !== side) sel.value = side;
  const toolsLeft = toolsOnLeft();
  listeners.forEach(cb => { try { cb(side, toolsLeft); } catch (_) { /* 1 つの失敗で他を止めない */ } });
}

/** ⇄ ボタン用。今いる側の反対へ倒す（既定のまま一度も倒していなければ左へ）。 */
export function toggleUiSide(): void {
  setUiSide(toolsOnLeft() ? 'right' : 'left');
}

/** 側が変わったときに呼ばれる。DOM 並べ替えが要る入力欄ツール用。 */
export function onUiSideChange(cb: Listener): void {
  listeners.push(cb);
}

export function initUiSide(): void {
  applyBodyClasses(getUiSide());

  const sel = document.getElementById('ui-side-select') as HTMLSelectElement | null;
  if (!sel) return;
  sel.value = getUiSide();
  sel.addEventListener('change', () => {
    const v = sel.value;
    setUiSide(v === 'left' || v === 'right' ? v : '');
  });
}

// UI の操作系を「要素ごと」にどちらの側へ置くかを持つ。
//
// 対象は 4 つ:
//   - cards    セッションカードの列（#session-list）
//   - tools    入力欄のツール群（#input-wrap 内のボタン列。⇄ = #tools-flip-btn）
//   - close    各パネルの閉じるボタン（Review / Git / 設定 / 各モーダルの ✕）
//   - approval 確認ポップアップ（#action-bar）。軸が違うため '' = 下ドック
//
// v0.8 までは uiSide 1 本で 3 要素をまとめて倒していた（32f101b）。
// 「カードは左・ポップアップは右・入力欄は右」のように要素ごとに決めたい要求のため、
// localStorage キーを要素ごとの 4 本へ分けた。⇄ ボタンは 4 要素を一括で倒す
// ショートカットとして残す。
//
// 値は '' / 'left' / 'right' の 3 値。'' は「その要素の従来どおりの位置」を意味し、
// 更新しただけで画面が変わらないことを守る:
//   cards '' = 左 / tools '' = 旧キー ai_cli_hub_tools_left /
//   close '' = 右 / approval '' = 下ドック
//
// 設計メモ:
//  - DOM は動かさず flex の order だけを入れ替える（DOM を並べ替えると detached-grid の
//    要素隠しやモバイルドロワーの前提が崩れる）。入力欄ツールだけは従来から DOM を
//    並べ替えているので、そちらは app.ts の applyToolsPosition に委ねる。
//  - モバイル幅ではカード列が position:fixed のドロワーになり order が効かないため、
//    カード列・確認ポップアップの入れ替えは CSS 側でデスクトップ幅にだけ効かせている。
//  - 保存先は localStorage（ブラウザ単位）。端末ごとに好みが違う設定なので
//    サーバー設定には持たせない（タブの並び順と同じ扱い）。

import { STORAGE_TOOLS_LEFT_KEY } from './user-prefs.js';

export type UiSide = '' | 'left' | 'right';

export type UiSideElement = 'cards' | 'tools' | 'close' | 'approval';

export type UiSideMap = Record<UiSideElement, UiSide>;

const STORAGE_KEYS: Record<UiSideElement, string> = {
  cards: 'uiSideCards',
  tools: 'uiSideTools',
  close: 'uiSideClose',
  approval: 'uiSideApproval',
};

const LEGACY_STORAGE_KEY = 'uiSide';
const ELEMENTS: readonly UiSideElement[] = ['cards', 'tools', 'close', 'approval'];

function normalize(v: string | null): UiSide {
  return v === 'left' || v === 'right' ? v : '';
}

/** 旧 uiSide（3 要素一括）からの移行。4 キーが 1 つも無いときだけ 1 回行う。 */
function migrateLegacy(): void {
  try {
    const hasAny = ELEMENTS.some((el) => localStorage.getItem(STORAGE_KEYS[el]) !== null);
    if (hasAny) return;
    const legacy = normalize(localStorage.getItem(LEGACY_STORAGE_KEY));
    if (!legacy) return;
    localStorage.setItem(STORAGE_KEYS.cards, legacy);
    localStorage.setItem(STORAGE_KEYS.tools, legacy);
    localStorage.setItem(STORAGE_KEYS.close, legacy);
    // approval は軸が違うので写さない（下ドックのまま）
    localStorage.removeItem(LEGACY_STORAGE_KEY);
  } catch (_) { /* private mode 等は無視 */ }
}

export function getUiSideEl(el: UiSideElement): UiSide {
  try {
    return normalize(localStorage.getItem(STORAGE_KEYS[el]));
  } catch (_) {
    return '';
  }
}

export function getUiSides(): UiSideMap {
  return {
    cards: getUiSideEl('cards'),
    tools: getUiSideEl('tools'),
    close: getUiSideEl('close'),
    approval: getUiSideEl('approval'),
  };
}

/** 入力欄ツールを左に置くか。既定（''）のときだけ従来キーを見る。 */
export function toolsOnLeft(): boolean {
  const side = getUiSideEl('tools');
  if (side) return side === 'left';
  try { return localStorage.getItem(STORAGE_TOOLS_LEFT_KEY) === '1'; } catch (_) { return false; }
}

/** 確認ポップアップが縦カラム（右 or 左）なら true。 */
export function isApprovalColumn(): boolean {
  return getUiSideEl('approval') !== '';
}

function applyBodyClasses(state: UiSideMap): void {
  document.body.classList.toggle('sidebar-right', state.cards === 'right');
  document.body.classList.toggle('close-left', state.close === 'left');
  document.body.classList.toggle('approval-dock-right', state.approval === 'right');
  document.body.classList.toggle('approval-dock-left', state.approval === 'left');
}

export function setUiSideEl(el: UiSideElement, side: UiSide): void {
  try {
    if (side) localStorage.setItem(STORAGE_KEYS[el], side);
    else localStorage.removeItem(STORAGE_KEYS[el]);
    // 入力欄ツールは旧キーも同じ値に保つ（読む側が増えても状態が割れないように）
    if (el === 'tools') localStorage.setItem(STORAGE_TOOLS_LEFT_KEY, side === 'left' ? '1' : '0');
  } catch (_) { /* private mode 等は無視 */ }
  const state = getUiSides();
  applyBodyClasses(state);
  syncSelects(state);
  listeners.forEach(cb => { try { cb(state, toolsOnLeft()); } catch (_) { /* 1 つの失敗で他を止めない */ } });
}

function syncSelects(state: UiSideMap): void {
  for (const el of ELEMENTS) {
    const sel = document.getElementById(`ui-side-${el}-select`) as HTMLSelectElement | null;
    if (sel && sel.value !== state[el]) sel.value = state[el];
  }
}

/** ⇄ ボタン用。入力欄ツールの実効側の反対へ 4 要素を一括で倒す。
 *  下ドック（''）の確認ポップアップは下のまま動かさない。 */
export function toggleUiSide(): void {
  const target: UiSide = toolsOnLeft() ? 'right' : 'left';
  for (const el of ELEMENTS) {
    if (el === 'approval') continue;
    setUiSideEl(el, target);
  }
  if (getUiSideEl('approval')) setUiSideEl('approval', target);
}

/** 側が変わったときに呼ばれる。DOM 並べ替えが要る入力欄ツール用。 */
type Listener = (state: UiSideMap, toolsLeft: boolean) => void;
const listeners: Listener[] = [];
export function onUiSideChange(cb: Listener): void {
  listeners.push(cb);
}

export function initUiSide(): void {
  migrateLegacy();
  const state = getUiSides();
  applyBodyClasses(state);

  for (const el of ELEMENTS) {
    const sel = document.getElementById(`ui-side-${el}-select`) as HTMLSelectElement | null;
    if (!sel) continue;
    sel.value = state[el];
    sel.addEventListener('change', () => {
      setUiSideEl(el, sel.value === 'left' || sel.value === 'right' ? (sel.value as UiSide) : '');
    });
  }
}

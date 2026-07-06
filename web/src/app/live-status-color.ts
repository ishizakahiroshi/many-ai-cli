// ライブステータス帯（#terminal-live-status「実行中」）のユーザー配色。
// 帯の右端にパレットボタンを置き、背景色・文字色をネイティブのカラーピッカーで
// 自由に変えられるようにする。設定は user-prefs 経由でサーバ同期する（全端末共通）。
//
// 適用方式: documentElement に CSS 変数 --live-status-bg / --live-status-fg を立て、
// terminal.css 側の #terminal-live-status がそれを参照する（未設定時は既定の青系に
// フォールバック）。idle/waiting 状態は意味づけのため独自の文字色を上書きするので、
// ユーザー配色が最も効くのは active（実行中）表示時。

import {
  STORAGE_LIVE_STATUS_BG_KEY,
  STORAGE_LIVE_STATUS_FG_KEY,
  setUserPref,
} from './user-prefs.js';

// 既定値（terminal.css の #terminal-live-status と揃える）。カラーピッカーの初期表示に使う。
const DEFAULT_BG = '#0d1117';
const DEFAULT_FG = '#93c5fd';

function ti18n(key: string): string {
  const tfn = typeof window.t === 'function' ? window.t : (k: string) => k;
  return tfn(key);
}

function isHexColor(v: string | null | undefined): v is string {
  return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v);
}

function readStored(key: string): string {
  try {
    const v = localStorage.getItem(key);
    return isHexColor(v) ? v! : '';
  } catch (_) {
    return '';
  }
}

// localStorage の現在値を CSS 変数へ反映する（未設定なら変数を消して既定へフォールバック）。
export function applyLiveStatusColors(): void {
  const root = document.documentElement;
  const bg = readStored(STORAGE_LIVE_STATUS_BG_KEY);
  const fg = readStored(STORAGE_LIVE_STATUS_FG_KEY);
  if (bg) root.style.setProperty('--live-status-bg', bg);
  else root.style.removeProperty('--live-status-bg');
  if (fg) root.style.setProperty('--live-status-fg', fg);
  else root.style.removeProperty('--live-status-fg');
}

function saveColor(which: 'bg' | 'fg', hex: string): void {
  const key = which === 'bg' ? STORAGE_LIVE_STATUS_BG_KEY : STORAGE_LIVE_STATUS_FG_KEY;
  try { localStorage.setItem(key, hex); } catch (_) {}
  setUserPref(which === 'bg' ? 'display.live_status_bg' : 'display.live_status_fg', hex);
  applyLiveStatusColors();
}

function resetColors(): void {
  try {
    localStorage.removeItem(STORAGE_LIVE_STATUS_BG_KEY);
    localStorage.removeItem(STORAGE_LIVE_STATUS_FG_KEY);
  } catch (_) {}
  // 空文字を送ってサーバ側もクリア（String('') → 削除扱い）
  setUserPref('display.live_status_bg', '');
  setUserPref('display.live_status_fg', '');
  applyLiveStatusColors();
}

let popoverEl: HTMLElement | null = null;
let paletteButtonEl: HTMLElement | null = null;
let longPressTimer: ReturnType<typeof setTimeout> | null = null;
let suppressDocumentClickUntil = 0;

const mobileMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)') : null;

function isMobileViewport(): boolean {
  return !!mobileMql?.matches;
}

function closePopover(): void {
  if (popoverEl) popoverEl.hidden = true;
}

function openPalette(anchor: HTMLElement, syncInputs: () => void, positionPopover: (anchor: HTMLElement) => void): void {
  if (!popoverEl) return;
  syncInputs();
  popoverEl.hidden = false;
  positionPopover(anchor);
}

function buildUI(): void {
  const bar = document.getElementById('terminal-live-status');
  if (!bar || bar.querySelector('.live-status-palette-btn')) return;

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'live-status-palette-btn';
  btn.textContent = '🎨';
  btn.setAttribute('aria-label', ti18n('live_status_palette_tooltip'));
  btn.title = ti18n('live_status_palette_tooltip');
  paletteButtonEl = btn;

  const pop = document.createElement('div');
  pop.className = 'live-status-palette-popover';
  pop.hidden = true;

  const bgRow = document.createElement('label');
  bgRow.className = 'live-status-palette-row';
  const bgText = document.createElement('span');
  bgText.textContent = ti18n('live_status_palette_bg');
  const bgInput = document.createElement('input');
  bgInput.type = 'color';
  bgRow.append(bgText, bgInput);

  const fgRow = document.createElement('label');
  fgRow.className = 'live-status-palette-row';
  const fgText = document.createElement('span');
  fgText.textContent = ti18n('live_status_palette_fg');
  const fgInput = document.createElement('input');
  fgInput.type = 'color';
  fgRow.append(fgText, fgInput);

  const resetBtn = document.createElement('button');
  resetBtn.type = 'button';
  resetBtn.className = 'live-status-palette-reset';
  resetBtn.textContent = ti18n('live_status_palette_reset');

  pop.append(bgRow, fgRow, resetBtn);
  bar.append(btn);
  // ポップオーバーは帯（#terminal-live-status: overflow:hidden）に入れると
  // 帯の枠外へ出た瞬間クリップされて見えなくなる。body 直下に出し、開くたびに
  // ボタン位置から position:fixed で配置する。
  document.body.append(pop);
  popoverEl = pop;

  const syncInputs = () => {
    bgInput.value = readStored(STORAGE_LIVE_STATUS_BG_KEY) || DEFAULT_BG;
    fgInput.value = readStored(STORAGE_LIVE_STATUS_FG_KEY) || DEFAULT_FG;
  };

  // ボタンの右上にポップオーバーを配置（帯の上側に出す）。
  const positionPopover = (anchor: HTMLElement = btn) => {
    const r = anchor.getBoundingClientRect();
    pop.style.left = 'auto';
    pop.style.right = `${Math.max(6, window.innerWidth - r.right)}px`;
    // 一旦表示してから高さを測り、ボタンの上に重ならないよう持ち上げる。
    pop.style.bottom = `${Math.max(6, window.innerHeight - r.top + 4)}px`;
    pop.style.top = 'auto';
  };

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (pop.hidden) openPalette(btn, syncInputs, positionPopover);
    else pop.hidden = true;
  });

  const cancelLongPress = () => {
    if (longPressTimer !== null) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
    }
  };
  bar.addEventListener('touchstart', (e) => {
    if (!isMobileViewport()) return;
    if (e.target === btn || pop.contains(e.target as Node)) return;
    cancelLongPress();
    longPressTimer = setTimeout(() => {
      longPressTimer = null;
      openPalette(bar, syncInputs, positionPopover);
      suppressDocumentClickUntil = Date.now() + 450;
    }, 500);
  }, { passive: true });
  bar.addEventListener('touchmove', cancelLongPress, { passive: true });
  bar.addEventListener('touchend', cancelLongPress, { passive: true });
  bar.addEventListener('touchcancel', cancelLongPress, { passive: true });
  // 入力中（input）でライブプレビュー、確定（change）で保存
  bgInput.addEventListener('input', () => document.documentElement.style.setProperty('--live-status-bg', bgInput.value));
  fgInput.addEventListener('input', () => document.documentElement.style.setProperty('--live-status-fg', fgInput.value));
  bgInput.addEventListener('change', () => saveColor('bg', bgInput.value));
  fgInput.addEventListener('change', () => saveColor('fg', fgInput.value));
  resetBtn.addEventListener('click', (e) => { e.stopPropagation(); resetColors(); syncInputs(); });

  pop.addEventListener('click', (e) => e.stopPropagation());
  document.addEventListener('click', (e) => {
    if (Date.now() < suppressDocumentClickUntil) return;
    if (!pop.hidden && e.target !== btn && !pop.contains(e.target as Node)) closePopover();
  });
}

export function initLiveStatusColor(): void {
  applyLiveStatusColors();
  buildUI();
}

export function openLiveStatusPalette(): void {
  if (!popoverEl || !paletteButtonEl) return;
  paletteButtonEl.click();
}

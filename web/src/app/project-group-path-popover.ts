// サイドバーの箱の見出し名にマウスを載せたときの吹き出し。名前の全文とフォルダのフルパスを出し、
// 📋 でフルパスをコピーできる。
//
// グローバルツールチップ（settings.ts の #hub-tooltip）はマウスに付いて動き、pointer-events も
// 切っているので中のボタンを押せない。そのため名前の下に固定して出す別の吹き出しにしている。
// 名前から吹き出しへポインタを移す間に消えないよう、消すのは少し待つ。
// タッチ操作には出さない（ホバーが無く、タップは見出しの「箱を開く」に使っている）。
//
// コピーの操作は 📋 だけにする。名前の上で Ctrl+C を拾う形にすると、端末にフォーカスが
// あるときの Ctrl+C（中断）を横取りしてしまう。

import { t } from '../i18n.js';
import { COPY_ICON, flashCopied } from './util.js';

const SHOW_DELAY_MS = 400;
// 名前から吹き出しへポインタを移す間は消さずに待つ。
const HIDE_DELAY_MS = 350;
const EDGE_MARGIN = 6;

let pop: HTMLDivElement | null = null;
let showTimer = 0;
let hideTimer = 0;

function isOpen(): boolean {
  return !!pop && pop.style.display === 'block';
}

function hideNow(): void {
  window.clearTimeout(showTimer);
  window.clearTimeout(hideTimer);
  if (pop) pop.style.display = 'none';
}

function scheduleHide(): void {
  window.clearTimeout(hideTimer);
  hideTimer = window.setTimeout(hideNow, HIDE_DELAY_MS);
}

function ensurePopover(): HTMLDivElement {
  if (pop) return pop;
  const el = document.createElement('div');
  el.className = 'project-path-pop';
  // 端末に重なることがあるので、ホイールを背後の xterm へ回さない（terminal.ts の除外）。
  el.dataset.wheelNative = '';
  el.addEventListener('pointerenter', () => window.clearTimeout(hideTimer));
  el.addEventListener('pointerleave', scheduleHide);
  document.body.appendChild(el);
  // 見出しを押した・ドラッグを始めた・スクロールした・Esc で閉じる。吹き出しの中を押したときは残す。
  document.addEventListener('pointerdown', (e) => {
    if (isOpen() && !el.contains(e.target as Node)) hideNow();
    else if (!isOpen()) window.clearTimeout(showTimer);
  }, true);
  document.addEventListener('dragstart', hideNow, true);
  document.addEventListener('scroll', (e) => {
    if (isOpen() && !el.contains(e.target as Node)) hideNow();
  }, true);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && isOpen()) hideNow(); });
  window.addEventListener('blur', hideNow);
  pop = el;
  return el;
}

// 縦は見出し全体の下に出し、数字や操作ボタンを覆わない。
function position(el: HTMLElement, anchor: HTMLElement): void {
  const r = anchor.getBoundingClientRect();
  const box = (anchor.closest('.project-group-header') as HTMLElement | null)?.getBoundingClientRect() ?? r;
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const left = Math.max(EDGE_MARGIN, Math.min(r.left - 4, window.innerWidth - w - EDGE_MARGIN));
  let top = box.bottom + 2;
  if (top + h > window.innerHeight - EDGE_MARGIN) top = Math.max(EDGE_MARGIN, box.top - h - 2);
  el.style.left = `${Math.round(left)}px`;
  el.style.top = `${Math.round(top)}px`;
}

function show(anchor: HTMLElement, name: string, path: string): void {
  if (!anchor.isConnected) return;
  const el = ensurePopover();
  el.textContent = '';

  const nameEl = document.createElement('div');
  nameEl.className = 'project-path-pop-name';
  nameEl.textContent = name;
  el.appendChild(nameEl);

  const row = document.createElement('div');
  row.className = 'project-path-pop-row';
  const pathEl = document.createElement('span');
  pathEl.className = 'project-path-pop-path';
  pathEl.textContent = path;
  row.appendChild(pathEl);
  const copyBtn = document.createElement('button');
  copyBtn.type = 'button';
  copyBtn.className = 'ui-button ui-button--quiet ui-button--compact project-path-pop-copy';
  copyBtn.textContent = COPY_ICON;
  copyBtn.title = t('copy_to_clipboard');
  copyBtn.setAttribute('aria-label', `${t('copy_to_clipboard')}: ${path}`);
  copyBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    navigator.clipboard?.writeText(path).then(() => flashCopied(copyBtn)).catch(() => {});
  });
  row.appendChild(copyBtn);
  el.appendChild(row);

  el.style.display = 'block';
  position(el, anchor);
}

/**
 * Windows のドライブ付きパスは区切りを \ に揃える（git が / で返すことがあるため、
 * エクスプローラーやシェルへそのまま貼れる形にする）。それ以外はそのまま返す。
 */
export function displayProjectPath(path: string): string {
  return /^[A-Za-z]:[\\/]/.test(path) ? path.replace(/\//g, '\\') : path;
}

/** 見出し名の要素に吹き出しを付ける。path が空なら何も付けない。 */
export function attachProjectPathPopover(anchor: HTMLElement, name: string, path: string): void {
  if (!path) return;
  const shown = displayProjectPath(path);
  anchor.addEventListener('pointerenter', (e) => {
    if (e.pointerType !== 'mouse') return;
    window.clearTimeout(hideTimer);
    window.clearTimeout(showTimer);
    // 別の見出しから移ってきたときは待たずに切り替える。
    showTimer = window.setTimeout(() => show(anchor, name, shown), isOpen() ? 0 : SHOW_DELAY_MS);
  });
  anchor.addEventListener('pointerleave', () => {
    window.clearTimeout(showTimer);
    if (isOpen()) scheduleHide();
  });
}

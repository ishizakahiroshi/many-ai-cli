// URL リンクの共通部品。URL の判定は url-detect.ts、ここは DOM とクリック時の動きだけを持つ。
//
// クリックしてもすぐには開かず、パスのリンクと同じポップアップに URL の全文と
// 「ブラウザで開く」「URL をコピー」を出す。AI が出した URL を 1 クリックで開かせない
// （prompt injection 経由のフィッシング対策・2026-07 監査の H5）ための決めで、
// 端末・ファイルプレビュー・チャット履歴などどの画面でも同じ動きにする。
// 画面上の <a> のリンクでは、中クリック・右クリックはブラウザ標準の動き（新しいタブ・リンクのコピー）のまま。
import {
  appendLinkedText,
  cancelPathPopupHideTimer,
  copyPathText,
  getOrCreatePathPopup,
  renderPathPopupItems,
} from './path-links.js';
import { findUrlCandidates, isHttpUrl, splitUrlPieces } from './url-detect.js';

export function showUrlPopup(url: string, clientX: number, clientY: number): void {
  if (!isHttpUrl(url)) return;
  cancelPathPopupHideTimer();
  const popup = getOrCreatePathPopup();
  popup.innerHTML = '';
  popup.hidden = false;
  const head = document.createElement('div');
  head.className = 'path-link-popup-url';
  head.textContent = url;
  popup.appendChild(head);
  renderPathPopupItems(popup, [
    { icon: '🌐', key: 'link_open_url', action: () => { window.open(url, '_blank', 'noopener,noreferrer'); } },
    { icon: '📋', key: 'link_copy_url', action: (anchor) => copyPathText(url, anchor).catch(() => {}) },
  ], clientX, clientY);
}

// <a> のクリックを showUrlPopup へ向ける。キーボード（Enter）で押されたときは座標が 0 なので、
// リンク自身の位置にポップアップを出す。
export function bindUrlLink(a: HTMLAnchorElement, url: string): void {
  if (a.dataset.urlLinkBound) return;
  a.dataset.urlLinkBound = '1';
  a.addEventListener('click', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    let x = e.clientX;
    let y = e.clientY;
    if (x === 0 && y === 0) {
      const r = a.getBoundingClientRect();
      x = r.left;
      y = r.bottom;
    }
    showUrlPopup(url, x, y);
  });
}

export function createUrlLink(url: string, text: string = url, className = 'url-link'): HTMLAnchorElement {
  const a = document.createElement('a');
  a.className = className;
  a.href = url;
  a.target = '_blank';
  a.rel = 'noopener noreferrer';
  a.title = url;
  a.textContent = text;
  bindUrlLink(a, url);
  return a;
}

// text の URL を <a> にして parent へ足す。URL 以外の区間は appendPlain に任せる
// （省略時はそのまま文字として足す）。
export function appendTextWithUrlLinks(
  parent: Node,
  text: string,
  appendPlain?: (parent: Node, text: string) => void,
  className = 'url-link',
): void {
  const src = String(text || '');
  const plain = appendPlain || ((p: Node, s: string) => { p.appendChild(document.createTextNode(s)); });
  let pos = 0;
  for (const c of findUrlCandidates(src)) {
    if (c.start > pos) plain(parent, src.slice(pos, c.start));
    parent.appendChild(createUrlLink(c.url, c.url, className));
    pos = c.end;
  }
  if (pos < src.length) plain(parent, src.slice(pos));
}

// URL とファイルパスの両方をリンクにする（パスは path-links.appendLinkedText の動き）。
// container の中身は置き換える。
export function appendTextWithLinks(container: HTMLElement, text: string, sessionId: any, urlClassName = 'url-link'): void {
  container.textContent = '';
  appendTextWithUrlLinks(container, text, (parent, segment) => {
    // appendLinkedText は渡した要素を空にしてから埋めるため、区間ごとに一時要素へ書いて移す
    const span = document.createElement('span');
    appendLinkedText(span, segment, sessionId);
    while (span.firstChild) parent.appendChild(span.firstChild);
  }, urlClassName);
}

// 描画済みの要素（シンタックスハイライト済みの <pre> など）の中の URL をリンクにする。
// ハイライトで URL が複数の <span> に割れることがあるので、root 内の文字を 1 本につないで判定し、
// 割れた URL はかけらごとに同じ行き先の <a> で包む。つなぐときに区切りを入れないので、
// ブロック要素が並ぶ要素（Markdown の本文など）ではなく <pre> のような連続した文字に使う。
// 既存の <a> / ボタン / 検索対象外の要素の中は触らない。
export function linkifyUrlsInElement(root: Element, className = 'url-link'): void {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(n) {
      const parent = n.parentElement;
      if (parent && parent.closest('a, button, [data-files-skip-search]')) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  const nodes: Text[] = [];
  let node: Node | null;
  while ((node = walker.nextNode())) nodes.push(node as Text);
  const values = nodes.map((n) => n.nodeValue || '');
  const piecesByNode = splitUrlPieces(values);

  for (let i = 0; i < nodes.length; i++) {
    const pieces = piecesByNode[i];
    if (pieces.length === 0) continue;
    const textNode = nodes[i];
    const value = values[i];
    const frag = document.createDocumentFragment();
    let pos = 0;
    for (const p of pieces) {
      if (p.from > pos) frag.appendChild(document.createTextNode(value.slice(pos, p.from)));
      frag.appendChild(createUrlLink(p.url, value.slice(p.from, p.to), className));
      pos = p.to;
    }
    if (pos < value.length) frag.appendChild(document.createTextNode(value.slice(pos)));
    textNode.parentNode?.replaceChild(frag, textNode);
  }
}

// すでに <a href="https://…"> になっているリンク（Markdown の変換結果など）のクリックを
// showUrlPopup へ向ける。http(s) 以外の href と、処理済みの <a> は触らない。
export function bindUrlLinksIn(root: Element): void {
  root.querySelectorAll('a[href]').forEach((el) => {
    const a = el as HTMLAnchorElement;
    const href = a.getAttribute('href') || '';
    if (!isHttpUrl(href) || a.dataset.urlLinkBound) return;
    if (!a.title) a.title = href;
    bindUrlLink(a, href);
  });
}

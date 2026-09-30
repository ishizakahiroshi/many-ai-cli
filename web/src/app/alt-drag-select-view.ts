// alt-drag-select-view.ts — 代替画面の CLI（全画面モードの Claude Code 等）で、ドラッグ選択が
// 端末の上下端を越えたら CLI へ 1 段ずつスクロールを送り、画面の外へ出た選択の行を覚えておく。
//
// 突き合わせの計算は alt-drag-select.ts（純関数・fixtures 済み）。ここは入力の監視・送信・
// xterm の選択の起点の付け替え・コピー文字列の組み立て・案内の表示だけを持つ。
//
// 送るのは疑似レールと同じ scrollAltBufferPage()（1 段ずつ、画面の変化を確かめてから次）。
// 遡り位置（altScrollNotchesUp）もそちらで数えるので、承認の「遡り中」判定と食い違わない。
//
// xterm の選択の起点は公開 API では動かせない（term.select() はドラッグ中のマウス処理を
// 外してしまう）ため、SelectionService の model を直接書き換える。vendored の xterm に
// その名前が無いときは、この機能だけを止めて従来どおりの選択にする。

import { t as ti18n } from '../i18n.js';
import { showToast } from './util.js';
import { canPageAltBuffer, scrollAltBufferPage } from './terminal.js';
import { hasPendingAltScrollNotch } from './alt-scroll-rail-view.js';
import { measureAltDragShift, stepAltDragAnchor, type AltDragAnchor } from './alt-drag-select.js';

/** ポインタが端の外にある間、次の 1 段を送れるか確かめる間隔（ms）。xterm の DRAG_SCROLL_INTERVAL と同じ。 */
const TICK_MS = 50;
/** 1 段送ってから画面を読むまでの最短の待ち（ms）。CLI が 1 画面を描き終えるのを待つ。 */
const SETTLE_MS = 80;
/** 送っても画面が動かなかった回数がこれに達したら、ポインタが端の内側へ戻るまで送らない（履歴の端）。 */
const MAX_STILL = 3;

/** 画面の外へ出た、選択に入る行。above は見えている選択の前、below は後ろにつなぐ。 */
interface Carry {
  above: string[];
  below: string[];
}

interface InFlight {
  before: string[];
  anchor: AltDragAnchor;
  anchorPart: string;
  sentAt: number;
}

interface Drag {
  id: number;
  t: any;
  doc: Document;
  /** ポインタの位置。-1 = 画面の上端より上 / 1 = 下端より下 / 0 = 内側。 */
  edge: number;
  /** 最初に送った方向。1 回のドラッグの中では逆向きに送らない（覚えた行の前後が崩れるため）。 */
  direction: number;
  inFlight: InFlight | null;
  still: number;
  released: boolean;
  timer: ReturnType<typeof setInterval> | null;
  onMove: (e: MouseEvent) => void;
  onUp: (e: MouseEvent) => void;
}

const carries = new Map<number, Carry>();
const badges = new Map<number, HTMLElement>();
const installed = new WeakSet<HTMLElement>();
let drag: Drag | null = null;

function selectionModel(term: any): { svc: any; model: any } | null {
  const svc = term?._core?._selectionService;
  const model = svc?._model;
  if (!model || typeof svc.refresh !== 'function') return null;
  return { svc, model };
}

function readAnchor(term: any): AltDragAnchor | null {
  const start = selectionModel(term)?.model.selectionStart;
  if (!Array.isArray(start) || !term.hasSelection()) return null;
  return { col: start[0], row: start[1] };
}

function writeAnchor(term: any, anchor: AltDragAnchor): void {
  const m = selectionModel(term);
  if (!m) return;
  m.model.selectionStart = [anchor.col, anchor.row];
  m.model.selectionStartLength = 0;
  m.svc.refresh();
}

// 代替画面は履歴を持たないので、バッファの行番号がそのまま画面の行番号になる。
function screenLines(term: any): string[] {
  const buffer = term.buffer.active;
  const lines: string[] = [];
  for (let row = 0; row < term.rows; row++) {
    lines.push(buffer.getLine(row)?.translateToString(true) ?? '');
  }
  return lines;
}

// 起点の行のうち選択に入る部分。下へ伸ばす選択は起点から行末、上へ伸ばす選択は
// 行頭から起点の手前まで（xterm は逆向きの選択で起点の桁を含めない）。
function anchorPartOf(term: any, anchor: AltDragAnchor, direction: number): string {
  const line = term.buffer.active.getLine(anchor.row);
  if (!line) return '';
  return direction > 0 ? line.translateToString(true, anchor.col) : line.translateToString(true, 0, anchor.col);
}

function edgeOf(term: any, clientY: number): number {
  const screen = (term.element as HTMLElement | null)?.querySelector('.xterm-screen');
  if (!screen) return 0;
  const rect = screen.getBoundingClientRect();
  if (clientY > rect.bottom) return 1;
  if (clientY < rect.top) return -1;
  return 0;
}

function carryFor(id: number): Carry {
  let carry = carries.get(id);
  if (!carry) {
    carry = { above: [], below: [] };
    carries.set(id, carry);
  }
  return carry;
}

function clearCarry(id: number): void {
  carries.delete(id);
  const badge = badges.get(id);
  if (badge) badge.hidden = true;
}

// 選択の続きは画面に見えないので、コピーに含まれることを端末の上（下）端に出す。
function renderBadge(id: number, t: any): void {
  const carry = carries.get(id);
  const host = t?.term?.element as HTMLElement | null;
  const count = carry ? carry.above.length + carry.below.length : 0;
  let badge = badges.get(id);
  if (!carry || count === 0 || !host) {
    if (badge) badge.hidden = true;
    return;
  }
  if (!badge || badge.ownerDocument !== host.ownerDocument) {
    badge?.remove();
    badge = host.ownerDocument.createElement('div');
    badge.className = 'alt-drag-carry';
    badges.set(id, badge);
  }
  if (badge.parentElement !== host) host.appendChild(badge);
  const above = carry.above.length > 0;
  badge.classList.toggle('is-below', !above);
  badge.textContent = ti18n(above ? 'alt_drag_carry_above' : 'alt_drag_carry_below', { n: count });
  badge.hidden = false;
}

function stopDrag(d: Drag): void {
  if (d.timer !== null) {
    clearInterval(d.timer);
    d.timer = null;
  }
  d.doc.removeEventListener('mousemove', d.onMove);
  d.doc.removeEventListener('mouseup', d.onUp);
  if (drag === d) drag = null;
}

function releaseDrag(d: Drag): void {
  d.released = true;
  d.doc.removeEventListener('mousemove', d.onMove);
  d.doc.removeEventListener('mouseup', d.onUp);
  if (!d.t?.term?.hasSelection()) clearCarry(d.id);
  // 送った 1 段の結果がまだなら、tick が読み終えてから止める。
  if (!d.inFlight) stopDrag(d);
}

// 画面の動きを突き合わせられなかった。覚えた行と見えている選択の間が欠けるので、
// 黙って欠けたままコピーさせず、選択ごと解いて知らせる。
function loseSelection(d: Drag): void {
  d.still = MAX_STILL;
  d.released = true;
  clearCarry(d.id);
  try { d.t.term.clearSelection(); } catch (_) { /* 破棄済みの端末なら何もしない */ }
  showToast(ti18n('alt_drag_select_lost'));
  stopDrag(d);
}

function settle(d: Drag): void {
  const flight = d.inFlight;
  d.inFlight = null;
  const term = d.t?.term;
  // 結果を待つ間に選択が解かれた（別の場所をクリックした等）なら、起点を付け替えない。
  if (!flight || !term || !term.hasSelection()) return;
  const after = screenLines(term);
  const shift = measureAltDragShift(flight.before, after, d.direction);
  if (shift.kind === 'still') {
    d.still++;
    return;
  }
  if (shift.kind === 'moved') {
    const step = stepAltDragAnchor(flight.before, after, d.direction, shift.lines, flight.anchor, flight.anchorPart, term.cols);
    if (step) {
      d.still = 0;
      if (step.leftLines.length > 0) {
        const carry = carryFor(d.id);
        if (d.direction > 0) carry.above.push(...step.leftLines);
        else carry.below.unshift(...step.leftLines);
        renderBadge(d.id, d.t);
      }
      writeAnchor(term, step.anchor);
      return;
    }
  }
  loseSelection(d);
}

function tick(d: Drag): void {
  const term = d.t?.term;
  if (!term) {
    stopDrag(d);
    return;
  }
  if (d.inFlight) {
    if (hasPendingAltScrollNotch(d.id) || Date.now() - d.inFlight.sentAt < SETTLE_MS) return;
    settle(d);
    if (drag !== d) return;
  }
  if (d.released) {
    stopDrag(d);
    return;
  }
  if (d.edge === 0 || (d.direction !== 0 && d.edge !== d.direction) || d.still >= MAX_STILL) return;
  if (!canPageAltBuffer(d.id, d.t) || hasPendingAltScrollNotch(d.id)) return;
  const anchor = readAnchor(term);
  if (!anchor) return;
  const direction = d.edge;
  const before = screenLines(term);
  const anchorPart = anchorPartOf(term, anchor, direction);
  if (!scrollAltBufferPage(d.id, d.t, direction)) return;
  d.direction = direction;
  d.inFlight = { before, anchor, anchorPart, sentAt: Date.now() };
}

function startDrag(id: number, t: any, doc: Document): void {
  const d: Drag = {
    id,
    t,
    doc,
    edge: 0,
    direction: 0,
    inFlight: null,
    still: 0,
    released: false,
    timer: null,
    onMove: (e: MouseEvent) => {
      if ((e.buttons & 1) === 0) {
        releaseDrag(d);
        return;
      }
      d.edge = edgeOf(t.term, e.clientY);
      // 内側へ戻ったら、履歴の端で止めた送りをもう一度試せるようにする。
      if (d.edge === 0) d.still = 0;
    },
    onUp: () => releaseDrag(d),
  };
  doc.addEventListener('mousemove', d.onMove);
  doc.addEventListener('mouseup', d.onUp);
  d.timer = setInterval(() => tick(d), TICK_MS);
  drag = d;
}

/** term.open() 後に呼ぶ（何度呼んでもよい）。端末の要素へドラッグの監視を付ける。 */
export function ensureAltDragSelect(id: number, t: any): void {
  const el = t?.term?.element as HTMLElement | null;
  if (!el || installed.has(el) || !selectionModel(t.term)) return;
  installed.add(el);
  el.addEventListener('mousedown', (e: MouseEvent) => {
    if (e.button !== 0) return;
    const target = e.target as Element | null;
    if (target?.closest?.('.alt-scroll-rail, .scrollbar')) return;
    if (drag) stopDrag(drag);
    clearCarry(id);
    if (!canPageAltBuffer(id, t)) return;
    startDrag(id, t, el.ownerDocument);
  });
  t.term.onSelectionChange(() => {
    // ドラッグ中は起点の付け替えで一瞬だけ空の選択になることがあるので消さない。
    if (drag && drag.id === id && !drag.released) return;
    if (!t.term.hasSelection()) clearCarry(id);
  });
}

/**
 * コピー・送信に使う選択文字列。画面の外へ出た選択の行があれば、見えている選択の前後へつなぐ。
 * xterm の getSelection() の代わりに、選択をクリップボードや入力欄へ渡す箇所はすべてこれを使う。
 */
export function altDragSelectionText(id: number | null | undefined, term: any): string {
  const visible: string = term?.getSelection?.() ?? '';
  const carry = id == null ? undefined : carries.get(id);
  if (!visible || !carry || (carry.above.length === 0 && carry.below.length === 0)) return visible;
  // xterm は Windows で行を \r\n でつなぐので、見えている選択と同じ区切りにそろえる。
  const separator = visible.includes('\r\n') ? '\r\n' : '\n';
  return [...carry.above, visible, ...carry.below].join(separator);
}

/** セッション破棄時に呼ぶ。 */
export function disposeAltDragSelect(id: number): void {
  if (drag && drag.id === id) stopDrag(drag);
  carries.delete(id);
  const badge = badges.get(id);
  if (badge) {
    try { badge.remove(); } catch (_) { /* 既に外れていれば何もしない */ }
    badges.delete(id);
  }
}

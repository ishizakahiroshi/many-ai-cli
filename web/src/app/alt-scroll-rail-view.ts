// alt-scroll-rail-view.ts — 代替画面バッファのセッションへ出す疑似スクロールレール（DOM 側）。
//
// 位置モデルは alt-scroll-rail.ts（純関数・node:test 済み）。ここは描画と入力だけを持つ。
// なぜ必要かは alt-scroll-rail.ts の冒頭と docs/local/plan_alt-screen-scroll-rail.md を参照。
//
// 操作はすべて terminal.ts の scrollAltBufferPage() を経由して PgUp / PgDn として
// CLI へ送る。ホイールと ↑up / ↓down ボタンも同じ関数を通るので、3 経路のどれで
// スクロールしてもスライダー位置に反映される（計上は terminal.ts 側で行う）。

import { terminals } from './state.js';
import { t as ti18n } from '../i18n.js';
import {
  AltRailState,
  altRailApplyPages,
  altRailGeometry,
  altRailInitialState,
  altRailPagesFromSliderTop,
  altRailStep,
} from './alt-scroll-rail.js';
import { canPageAltBuffer, markTerminalManualScrollIntent, scrollAltBufferPage } from './terminal.js';

/** キーを流す間隔（ms）。1 ドラッグで PageUp を連投して TUI を壊さないための律速。 */
const PUMP_INTERVAL_MS = 40;
/** 1 回の操作で送るページ数の上限。grok の PageUp 連投事故と同型を作らないための蓋。 */
const MAX_QUEUED_PAGES = 30;

interface RailEntry {
  rail: HTMLElement;
  slider: HTMLElement;
  state: AltRailState;
  /** ドラッグ・クリックで目指しているページ数。未操作時は null。 */
  target: number | null;
  dragging: boolean;
  grabOffset: number;
  pumpTimer: ReturnType<typeof setInterval> | null;
}

const rails = new Map<number, RailEntry>();

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function displayState(entry: RailEntry): AltRailState {
  // ドラッグ中はキー送信の完了を待たずスライダーを指に追従させる。
  if (entry.target === null) return entry.state;
  return altRailApplyPages(entry.state, entry.target);
}

// PTY の flush ごとに clientHeight を読むとレイアウトを強制同期させてしまうため、
// 描画は rAF で 1 フレーム 1 回にまとめる（flush は 1 セッションで毎秒数百回来る）。
const pendingRender = new Set<number>();
let renderFrame: number | null = null;

function renderRail(id: number): void {
  pendingRender.add(id);
  if (renderFrame !== null) return;
  renderFrame = requestAnimationFrame(() => {
    renderFrame = null;
    const ids = Array.from(pendingRender);
    pendingRender.clear();
    for (const each of ids) renderRailNow(each);
  });
}

function renderRailNow(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  const t = terminals.get(id);
  const visible = !!t && canPageAltBuffer(id, t);
  entry.rail.hidden = !visible;
  if (!visible) return;
  const track = entry.rail.clientHeight;
  if (track <= 0) return;
  const { sliderTop, sliderHeight } = altRailGeometry(displayState(entry), track);
  entry.slider.style.top = `${sliderTop}px`;
  entry.slider.style.height = `${sliderHeight}px`;
}

function stopPump(entry: RailEntry): void {
  if (entry.pumpTimer !== null) {
    clearInterval(entry.pumpTimer);
    entry.pumpTimer = null;
  }
}

function pump(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  if (entry.target === null) {
    stopPump(entry);
    return;
  }
  const diff = entry.target - entry.state.pagesUp;
  if (diff === 0) {
    if (!entry.dragging) entry.target = null;
    stopPump(entry);
    renderRail(id);
    return;
  }
  const t = terminals.get(id);
  if (!t) {
    entry.target = null;
    stopPump(entry);
    return;
  }
  // 送ってもページ数が動かない状況（想定外のクランプ等）で回り続けないための保険。
  const before = entry.state.pagesUp;
  // diff > 0 は「もっと上へ」＝ PgUp。scrollAltBufferPage 側でページ数が計上される。
  const sent = scrollAltBufferPage(id, t, diff > 0 ? -1 : 1);
  // 転送できない provider・メインバッファへ戻った場合は諦める（レールも次の描画で消える）。
  if (!sent || entry.state.pagesUp === before) {
    entry.target = null;
    stopPump(entry);
    renderRail(id);
  }
}

function requestPages(id: number, target: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  entry.target = clamp(
    Math.max(0, Math.round(target)),
    entry.state.pagesUp - MAX_QUEUED_PAGES,
    entry.state.pagesUp + MAX_QUEUED_PAGES,
  );
  markTerminalManualScrollIntent();
  renderRail(id);
  if (entry.pumpTimer === null) {
    entry.pumpTimer = setInterval(() => pump(id), PUMP_INTERVAL_MS);
    pump(id);
  }
}

function sliderTopFromPointer(entry: RailEntry, clientY: number): number {
  const rect = entry.rail.getBoundingClientRect();
  return clientY - rect.top - entry.grabOffset;
}

function bindPointer(id: number, entry: RailEntry): void {
  entry.rail.addEventListener('pointerdown', (ev: PointerEvent) => {
    const t = terminals.get(id);
    if (!t) return;
    // xterm 側のテキスト選択を開始させない。
    ev.preventDefault();
    ev.stopPropagation();
    const sliderRect = entry.slider.getBoundingClientRect();
    if (ev.clientY >= sliderRect.top && ev.clientY <= sliderRect.bottom) {
      // 掴んだだけでは動かさない（掴み位置を保ったまま pointermove で追従させる）。
      entry.dragging = true;
      entry.grabOffset = ev.clientY - sliderRect.top;
      try { entry.rail.setPointerCapture(ev.pointerId); } catch (_) { /* 対応外環境では掴み替えのみ諦める */ }
      return;
    }
    // トラック部分のクリックは通常のスクロールバーと同じく 1 ページ送り。
    const base = displayState(entry);
    const up = ev.clientY < sliderRect.top;
    requestPages(id, altRailStep(base, up ? -1 : 1).pagesUp);
  });

  entry.rail.addEventListener('pointermove', (ev: PointerEvent) => {
    if (!entry.dragging) return;
    ev.preventDefault();
    const railRect = entry.rail.getBoundingClientRect();
    requestPages(id, altRailPagesFromSliderTop(entry.state, railRect.height, sliderTopFromPointer(entry, ev.clientY)));
  });

  const endDrag = (ev: PointerEvent) => {
    if (!entry.dragging) return;
    entry.dragging = false;
    try { entry.rail.releasePointerCapture(ev.pointerId); } catch (_) { /* capture 未取得なら何もしない */ }
    renderRail(id);
  };
  entry.rail.addEventListener('pointerup', endDrag);
  entry.rail.addEventListener('pointercancel', endDrag);
}

/** term.open() 後に 1 度だけ呼ぶ。レールを .xterm 直下へ差し込む。 */
export function ensureAltScrollRail(id: number, t: any): void {
  if (rails.has(id)) {
    // セッション切替・ペイン移動で element が作り直された場合だけ差し直す。
    const entry = rails.get(id)!;
    const host = t?.term?.element;
    if (host && entry.rail.parentElement !== host) host.appendChild(entry.rail);
    renderRail(id);
    return;
  }
  const host = t?.term?.element;
  if (!host) return;
  // 別窓 Session Grid は別ドキュメントで動くので、host 側の document から作る。
  const doc: Document = host.ownerDocument || document;
  const rail = doc.createElement('div');
  rail.className = 'alt-scroll-rail';
  rail.hidden = true;
  rail.setAttribute('role', 'scrollbar');
  rail.setAttribute('aria-orientation', 'vertical');
  rail.setAttribute('aria-label', ti18n('alt_scroll_rail_label'));
  const slider = doc.createElement('div');
  slider.className = 'alt-scroll-rail-slider';
  rail.appendChild(slider);
  host.appendChild(rail);
  const entry: RailEntry = {
    rail,
    slider,
    state: altRailInitialState(),
    target: null,
    dragging: false,
    grabOffset: 0,
    pumpTimer: null,
  };
  rails.set(id, entry);
  bindPointer(id, entry);
  renderRail(id);
}

/** PTY 出力の flush・fit・attach のたびに呼ぶ（表示条件と幾何の同期）。 */
export function updateAltScrollRail(id: number): void {
  if (!rails.has(id)) return;
  renderRail(id);
}

/**
 * PgUp / PgDn を 1 ページ送ったことを計上する。terminal.ts の scrollAltBufferPage から
 * 呼ばれるので、ホイール・↑up / ↓down ボタン・レールの 3 経路すべてが反映される。
 */
export function noteAltScrollPage(id: number, direction: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  entry.state = altRailStep(entry.state, direction);
  renderRail(id);
}

/** セッション破棄時に呼ぶ。 */
export function disposeAltScrollRail(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  stopPump(entry);
  try { entry.rail.remove(); } catch (_) { /* 既に外れていれば何もしない */ }
  rails.delete(id);
}

// alt-scroll-rail-view.ts — 代替画面バッファのセッションへ出す疑似スクロールレール（DOM 側）。
//
// 位置モデルは alt-scroll-rail.ts（純関数・node:test 済み）。ここは描画と入力だけを持つ。
// なぜ必要かは alt-scroll-rail.ts の冒頭と docs/local/archive/v0.8.x/plan_alt-screen-scroll-rail.md を参照。
//
// 操作はすべて terminal.ts の scrollAltBufferPage() を経由して 1 ノッチとして
// CLI へ送る。ホイール／メイン画面の ↑up・↓down ボタン／レール自身の pump／
// マルチペイン ↑up・↓down／別窓グリッド ↑up・↓down の 6 経路のどれで
// スクロールしても、PTY 応答後に画面変化を確認できた分だけ位置へ反映する。

import { terminals } from './state.js';
import { t as ti18n } from '../i18n.js';
import {
  AltRailState,
  altRailClampTarget,
  altRailGeometry,
  altRailInitialState,
  altRailNotchesFromSliderTop,
  altRailStep,
} from './alt-scroll-rail.js';
import {
  isConfirmedAltScreenChange,
  type TerminalScreenSnapshot,
} from './terminal-history-strategy.js';
import { canPageAltBuffer, markTerminalManualScrollIntent, scrollAltBufferPage } from './terminal.js';

/** イベントを流す間隔（ms）。1 ドラッグでホイールを連投して TUI を壊さないための律速。 */
const PUMP_INTERVAL_MS = 40;
/** 1 回の操作で送るノッチ数の上限。大きな連投で TUI を壊さないための蓋。 */
const MAX_QUEUED_NOTCHES = 120;
/** 入力後に画面変化を待つ上限。履歴端の空振りで pump を回し続けないための蓋。 */
const CONFIRM_TIMEOUT_MS = 350;

interface PendingNotch {
  direction: number;
  before: TerminalScreenSnapshot;
  timeout: ReturnType<typeof setTimeout>;
}

interface RailEntry {
  rail: HTMLElement;
  slider: HTMLElement;
  state: AltRailState;
  /** ドラッグ・クリックで目指しているノッチ数。未操作時は null。 */
  target: number | null;
  dragging: boolean;
  grabOffset: number;
  pumpTimer: ReturnType<typeof setInterval> | null;
  /** PTY へ送信済みで、xterm の可視画面変化を待っている 1 ノッチ。 */
  pending: PendingNotch | null;
}

const rails = new Map<number, RailEntry>();

function displayState(entry: RailEntry): AltRailState {
  // target は「移動要求」であって現在位置ではない。入力が履歴端で空振りしても
  // スライダーだけ先行しないよう、描画は確認済み state のみを正本にする。
  return entry.state;
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

function clearPending(entry: RailEntry): void {
  if (entry.pending === null) return;
  clearTimeout(entry.pending.timeout);
  entry.pending = null;
}

function stopUnconfirmedRequest(entry: RailEntry): void {
  clearPending(entry);
  entry.target = null;
  stopPump(entry);
}

function pump(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  if (entry.target === null) {
    stopPump(entry);
    return;
  }
  const diff = entry.target - entry.state.notchesUp;
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
  // diff > 0 は「もっと上へ」。位置の計上は PTY 応答後の confirm で行う。
  const sent = scrollAltBufferPage(id, t, diff > 0 ? -1 : 1);
  // 転送できない戦略・メインバッファへ戻った場合は諦める（レールも次の描画で消える）。
  if (!sent) {
    stopUnconfirmedRequest(entry);
    renderRail(id);
  }
}

export function requestNotches(id: number, target: number): boolean {
  const entry = rails.get(id);
  if (!entry) return false;
  const clamped = altRailClampTarget(entry.state, target, MAX_QUEUED_NOTCHES);
  // 既に目標どおりの位置で、かつ動いている最中でもないなら「何もしなかった」と正直に返す。
  // ドラッグ中に掴んだ位置へ戻す呼び出し（entry.target が既に非 null）は、動きを止める
  // ための正当な操作なのでここでは弾かない。
  if (entry.target === null && clamped === entry.state.notchesUp) return false;
  entry.target = clamped;
  markTerminalManualScrollIntent();
  renderRail(id);
  if (entry.pumpTimer === null) {
    entry.pumpTimer = setInterval(() => pump(id), PUMP_INTERVAL_MS);
    pump(id);
  }
  return true;
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
    // トラック部分のクリックは通常のスクロールバーと同じく 1 ノッチ送り。
    const base = displayState(entry);
    const up = ev.clientY < sliderRect.top;
    requestNotches(id, altRailStep(base, up ? -1 : 1).notchesUp);
  });

  entry.rail.addEventListener('pointermove', (ev: PointerEvent) => {
    if (!entry.dragging) return;
    ev.preventDefault();
    const railRect = entry.rail.getBoundingClientRect();
    requestNotches(id, altRailNotchesFromSliderTop(entry.state, railRect.height, sliderTopFromPointer(entry, ev.clientY)));
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
    pending: null,
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
 * 1 ノッチを送る直前に、現在の可視画面を pending として保持する。
 * pending 中の追加イベントは PTY へ送らず、最初の応答を待つ。
 */
export function beginAltScrollNotch(
  id: number,
  direction: number,
  before: TerminalScreenSnapshot,
): 'started' | 'busy' | 'untracked' {
  const entry = rails.get(id);
  if (!entry) return 'untracked';
  if (entry.pending !== null) return 'busy';
  const pending = {} as PendingNotch;
  pending.direction = direction;
  pending.before = before;
  pending.timeout = setTimeout(() => {
    if (entry.pending !== pending) return;
    // PTY 出力が無い、または出ても可視本文が同一なら履歴端の空振り。要求を破棄し、
    // 確認済み位置からレールを動かさない。
    stopUnconfirmedRequest(entry);
    renderRail(id);
  }, CONFIRM_TIMEOUT_MS);
  entry.pending = pending;
  return 'started';
}

/** sendText が失敗したとき、画面変化待ちを即座に破棄する。 */
export function cancelAltScrollNotch(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  clearPending(entry);
}

/**
 * xterm が PTY chunk を描画し終えた後に呼ぶ。送信前から可視本文が操作方向へ
 * ずれた場合だけ 1 ノッチを確定するので、履歴端の空振りで疑似レールだけが進まない。
 */
export function confirmAltScrollNotch(id: number, after: TerminalScreenSnapshot): boolean {
  const entry = rails.get(id);
  const pending = entry?.pending;
  if (!entry || !pending) return false;
  if (!isConfirmedAltScreenChange(pending.before, after, pending.direction)) return false;
  const direction = pending.direction;
  clearPending(entry);
  entry.state = altRailStep(entry.state, direction);
  renderRail(id);
  return true;
}

/** PTY flush のたびに可視行を走査せず、pending 中だけ snapshot を作るための軽量判定。 */
export function hasPendingAltScrollNotch(id: number): boolean {
  return rails.get(id)?.pending != null;
}

/**
 * 画面移動を確認できたホイール上から下を引いた純ノッチ数。
 * 0 が最下部＝CLI はライブの画面を描いている。
 *
 * 0 より大きい間、CLI の画面には過去の位置が載っている。Hub の承認検出は VT ミラー＝
 * 今の画面を見るので、この値は「画面に出ているものを今の承認として扱ってよいか」の
 * 判定にも使われる（terminal.ts の isTerminalShowingHistory 経由で approval-ui.ts）。
 * 承認側に別の計上を作らないため、遡り位置の正本はここ 1 本にする。
 */
export function altScrollNotchesUp(id: number): number {
  return rails.get(id)?.state.notchesUp ?? 0;
}

/** セッション破棄時に呼ぶ。 */
export function disposeAltScrollRail(id: number): void {
  const entry = rails.get(id);
  if (!entry) return;
  clearPending(entry);
  stopPump(entry);
  try { entry.rail.remove(); } catch (_) { /* 既に外れていれば何もしない */ }
  rails.delete(id);
}

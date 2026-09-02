// alt-scroll-rail.ts — 代替画面バッファのセッションに出す疑似スクロールレールの位置モデル。
//
// なぜ要るか: Claude / opencode の TUI は `ESC[?1049h` で代替画面バッファへ入る。
// 代替画面は仕様上スクロールバックを持たないため、xterm 6.0 のスクロールバーは
// `ScrollbarVisibilityController.setIsNeeded(false)` のまま `invisible`（`fade` なし）に
// 固定され、terminal.css の常時表示上書きが効かない。つまり本物のスクロールバーは
// 原理的に出ない。メインバッファに描く Codex だけが本物を持っている状態だった。
// 由来: docs/local/archive/v0.8.x/plan_alt-screen-scroll-rail.md
//
// このモジュールは「PTY 応答後に画面移動を確認できたホイールイベントのノッチ数」から
// スライダーの位置と高さを決める。CLI 内部のスクロール量を取得する手段は無いので、
// **位置は近似**であり
// 正確な現在地ではない。近似であることを承知のうえで、Codex と同じ操作感へ揃える
// ためのモデルとして置いている。
//
// DOM に触らない純関数だけを置く（node:test から検証するため）。描画と入力は
// alt-scroll-rail-view.ts が担当する。

export interface AltRailState {
  /** 画面移動を確認できたホイール上から下を引いた純ノッチ数。0 が最下部。 */
  notchesUp: number;
  /** これまでに到達した notchesUp の最大値。仮想スクロール範囲の広さになる。 */
  notchesUpMax: number;
}

/**
 * 仮想ノッチ総数の下限。まだ 1 度も遡っていないセッションでも
 * 「上に履歴がある」ことが分かる大きさのスライダーを出すために置く。
 * 48 なら 1 ノッチ 1〜3 行として、従来の 4 ページぶんと近い見え方になる。
 */
export const ALT_RAIL_MIN_VIRTUAL_NOTCHES = 48;

/** スライダーの最小高さ（px）。VS Code 由来の xterm スクロールバーと同じ下限。 */
export const ALT_RAIL_MIN_SLIDER_PX = 20;

/**
 * 到達済みノッチ数に対する仮想範囲の広げ方。
 *
 * トランスクリプトの総量は CLI からは取得できない。仮想範囲を「到達済み + 1」に
 * すると、最上部までドラッグした時点でスライダーが上端に張り付き、それ以上
 * 遡れなくなる（＝ドラッグでは 1 度に数ノッチしか戻れない）。到達済みより
 * 常に広い範囲を持たせることで、上端まで引くたびに範囲が 1.5 倍ずつ伸び、
 * 「まだ上がある」ことも見た目で分かる。無限スクロールの UI と同じ考え方。
 */
export const ALT_RAIL_HEADROOM_RATIO = 1.5;

export function altRailInitialState(): AltRailState {
  return { notchesUp: 0, notchesUpMax: 0 };
}

function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, value));
}

/** 仮想スクロール範囲のノッチ総数。上へ遡るほど広がる（無限スクロールと同じ挙動）。 */
export function altRailNotchTotal(state: AltRailState): number {
  const reached = Math.max(0, Math.floor(state.notchesUpMax));
  const withHeadroom = Math.max(reached + 2, Math.round(reached * ALT_RAIL_HEADROOM_RATIO) + 1);
  return Math.max(ALT_RAIL_MIN_VIRTUAL_NOTCHES, withHeadroom);
}

export interface AltRailGeometry {
  /** トラック上端からのスライダー上端位置（px）。 */
  sliderTop: number;
  sliderHeight: number;
}

export function altRailGeometry(state: AltRailState, trackHeight: number): AltRailGeometry {
  const track = Math.max(0, Math.floor(trackHeight));
  const total = altRailNotchTotal(state);
  const sliderHeight = Math.min(track, Math.max(ALT_RAIL_MIN_SLIDER_PX, Math.round(track / total)));
  const span = Math.max(0, track - sliderHeight);
  // notchesUp = 0 で最下部、notchesUp = total - 1 で最上部。
  const ratio = total <= 1 ? 0 : clamp(state.notchesUp / (total - 1), 0, 1);
  return { sliderTop: Math.round(span * (1 - ratio)), sliderHeight };
}

/**
 * ドラッグ中のスライダー上端位置から目標ノッチ数を求める。
 * view 側は「掴んだ位置からのオフセット」を引いた値を渡す（掴み位置を保つため）。
 */
export function altRailNotchesFromSliderTop(state: AltRailState, trackHeight: number, sliderTop: number): number {
  const track = Math.max(0, Math.floor(trackHeight));
  const total = altRailNotchTotal(state);
  const { sliderHeight } = altRailGeometry(state, track);
  const span = Math.max(0, track - sliderHeight);
  if (span <= 0 || total <= 1) return 0;
  const ratio = 1 - clamp(sliderTop, 0, span) / span;
  return clamp(Math.round(ratio * (total - 1)), 0, total - 1);
}

/** notchesUp を更新した新しい state を返す（notchesUpMax は縮まない）。 */
export function altRailApplyNotches(state: AltRailState, nextNotches: number): AltRailState {
  const notchesUp = Math.max(0, Math.round(Number.isFinite(nextNotches) ? nextNotches : 0));
  return { notchesUp, notchesUpMax: Math.max(state.notchesUpMax, notchesUp) };
}

/** direction < 0 で 1 ノッチ上、> 0 で 1 ノッチ下。 */
export function altRailStep(state: AltRailState, direction: number): AltRailState {
  return altRailApplyNotches(state, state.notchesUp + (direction < 0 ? 1 : -1));
}

/**
 * 要求されたノッチ数を、0 以上かつ現在値から `maxQueued` 以内へクランプする。
 * `requestNotches()` が「実際に動く目標か」を判定するための下ごしらえ。
 */
export function altRailClampTarget(state: AltRailState, target: number, maxQueued: number): number {
  const rounded = Math.max(0, Math.round(Number.isFinite(target) ? target : 0));
  return clamp(rounded, state.notchesUp - maxQueued, state.notchesUp + maxQueued);
}

// 入力欄まわりのボタン（送信・音声・テンプレート・クイックコマンドなど）の置き場所と大きさ。
//
// 2 つの置き方がある。
//  1. 進捗帯（#terminal-live-status）が見えている PC 幅: ボタンは帯の中に「順番」で並ぶ。
//     動かすのはボタンの右上の取っ手だけ。ドラッグして別のボタンの間へ落とすと、ほかのボタンが
//     滑って間をあけ、並び直る（スマホのアイコン並べ替えと同じ）。保存するのは順番だけで、
//     画面の座標は持たない（サイドバーの開閉や窓幅の変更でずれない）。帯の外へは出せない。
//     帯の中のメッセージ（左）とボタン群（右）の境目をドラッグすると、ボタン領域の幅も変えられる。
//  2. 帯が無い・スマホ幅: 従来どおりの自由配置（画面上の座標を保存）。
// どちらの置き方でも、ボタンの普通のクリックは各ボタンの既存の処理に任せる。取っ手は兄弟の
// ボタンなので、ラベルを差し替えても取っ手とそのリスナーは消えない。
// Ctrl+ホイール（macOS は ⌘+ホイール）はボタン 1 つだけを 50〜400% に拡大縮小する。
const KEY = 'inputIconPositionsV2';
const OLD_KEY = 'inputIconPositionsV1';
const SCALE_KEY = 'inputIconScalesV1';
const ORDER_KEY = 'inputIconOrderV1';
const WIDTH_KEY = 'inputActionsWidthV1';
export const MIN_ICON_SCALE = 0.5;
export const MAX_ICON_SCALE = 4;
export const INPUT_ICON_IDS = [
  'mobile-composer-attach-btn', 'mobile-keyboard-toggle',
  'input-clear-btn', 'send-btn', 'voice-btn', 'prompt-template-toggle',
  'voice-wakeword-btn', 'quick-clear-btn', 'quick-model-btn',
  'quick-cmd-btn-3', 'quick-cmd-btn-4', 'quick-cmd-btn-5',
  'quick-cmd-btn-6', 'quick-cmd-btn-7', 'quick-cmd-btn-8',
  'quick-cmd-btn-9', 'quick-cmd-btn-10',
  'slash-picker-btn', 'buf-clear-btn',
] as const;
const IDS = INPUT_ICON_IDS;
type IconId = typeof IDS[number];
export type { IconId };
export interface IconPoint { x: number; y: number }
type Positions = Partial<Record<IconId, IconPoint>>;
type Scales = Partial<Record<IconId, number>>;
let positions: Positions = {};
let scales: Scales = {};
/** 帯の中の並び（左→右・段の順）。保存に無い（あとから足された）ボタンは既定の位置で補う。 */
let order: IconId[] = [];
/** ドラッグ中だけ持つ、落としたときの並びの下見。確定するまで保存しない。 */
let dragOrder: IconId[] | null = null;
/** ドラッグ中のボタンが指に付いてくる位置（画面座標）。 */
let dragFollow: { id: IconId; x: number; y: number } | null = null;
/** 利用者が決めたボタン領域の幅（px）。null は自動。 */
let actionsWidth: number | null = null;
let lastReserved = 0;
const grips = new Map<IconId, HTMLButtonElement>();
let resizer: HTMLButtonElement | null = null;
let cancelDrag: (() => void) | null = null;
const desktopDock = () => window.matchMedia('(min-width: 721px)').matches;

/** ボタン領域の幅の下限。これより狭いとボタンが 1 個も入らない。 */
export const MIN_ACTIONS_WIDTH = 60;
/** メッセージ側に最低限残す幅。 */
export const MIN_STATUS_TEXT_WIDTH = 120;

/** Lay out the controls at the right of the status band, leaving its text readable.
 *  A row is as tall as its tallest (resized) control, and never shorter than an unscaled row.
 *  fixedWidth を渡すと、ボタン領域をその幅に固定する（既定は帯の幅から 180px を残した残り）。 */
export function layoutStatusControls(widths: number[], bandWidth: number, heights: number[] = [], fixedWidth?: number): {
  points: IconPoint[]; rows: number; reservedWidth: number; height: number;
} {
  const gap = 6;
  const available = fixedWidth !== undefined ? Math.max(1, fixedWidth - 18) : Math.max(1, bandWidth - 180);
  const points: IconPoint[] = [];
  const rows: { start: number; width: number; height: number }[] = [];
  for (let i = 0; i < widths.length; i++) {
    let row = rows[rows.length - 1];
    if (!row || row.width + gap + widths[i] > available) {
      row = { start: i, width: 0, height: 0 };
      rows.push(row);
    }
    row.width += (i === row.start ? 0 : gap) + widths[i];
    row.height = Math.max(row.height, heights[i] ?? 36);
  }
  let y = 5;
  rows.forEach((row, rowIndex) => {
    let x = bandWidth - 8 - row.width;
    const end = rows[rowIndex + 1]?.start ?? widths.length;
    for (let i = row.start; i < end; i++) {
      points[i] = { x, y };
      x += widths[i] + gap;
    }
    y += Math.max(38, row.height + 2);
  });
  const reservedWidth = rows.length
    ? (fixedWidth !== undefined ? fixedWidth : (rows[0]?.width ?? 0) + 18)
    : 0;
  return { points, rows: rows.length, reservedWidth, height: rows.length ? y - 1 : 0 };
}

/** 利用者が決めたボタン領域の幅を、帯の幅に収まる範囲へ丸める。 */
export function clampActionsWidth(width: number, bandWidth: number): number {
  const max = Math.max(MIN_ACTIONS_WIDTH, bandWidth - MIN_STATUS_TEXT_WIDTH);
  return Math.round(Math.max(MIN_ACTIONS_WIDTH, Math.min(max, width)));
}

export function parseActionsWidth(raw: string | null): number | null {
  if (raw === null || raw.trim() === '') return null;
  const value = Number(raw);
  return Number.isFinite(value) && value >= MIN_ACTIONS_WIDTH ? Math.round(value) : null;
}

/** 保存された並びから、知っているボタンだけを重複なしで取り出す。 */
export function parseIconOrder(raw: string | null): IconId[] {
  try {
    const value = JSON.parse(raw || '[]');
    if (!Array.isArray(value)) return [];
    const out: IconId[] = [];
    for (const id of value) {
      if (typeof id === 'string' && (IDS as readonly string[]).includes(id) && !out.includes(id as IconId)) {
        out.push(id as IconId);
      }
    }
    return out;
  } catch (_) { return []; }
}

/** 保存に無いボタン（あとから足されたもの）を、既定の並びで直前にあるボタンの後ろへ入れる。 */
export function completeIconOrder(saved: readonly IconId[]): IconId[] {
  const out = [...saved];
  IDS.forEach((id, i) => {
    if (out.includes(id)) return;
    const prev = IDS.slice(0, i).reverse().find(p => out.includes(p));
    out.splice(prev ? out.indexOf(prev) + 1 : 0, 0, id);
  });
  return out;
}

/** 従来の座標の保存から、帯の中の並びを作る（上の段から、同じ段は左から）。座標が無ければ空。 */
export function orderFromPositions(saved: Positions): IconId[] {
  const placed = IDS.filter(id => saved[id]);
  if (!placed.length) return [];
  placed.sort((a, b) => saved[a]!.y - saved[b]!.y);
  const rows: IconId[][] = [];
  let rowY = -Infinity;
  for (const id of placed) {
    const y = saved[id]!.y;
    if (!rows.length || y - rowY > 12) { rows.push([]); rowY = y; }
    rows[rows.length - 1].push(id);
  }
  return rows.flatMap(row => row.sort((a, b) => saved[a]!.x - saved[b]!.x));
}

export interface IconSlot { cx: number; cy: number }

/** ポインタに最も近いスロットの、前か後ろかを返す（落としたとき並びの何番目に入るか）。 */
export function insertionIndex(slots: readonly IconSlot[], px: number, py: number): number {
  if (!slots.length) return 0;
  let best = 0;
  let bestDistance = Infinity;
  slots.forEach((slot, i) => {
    const distance = (slot.cx - px) ** 2 + (slot.cy - py) ** 2;
    if (distance < bestDistance) { bestDistance = distance; best = i; }
  });
  return px >= slots[best].cx ? best + 1 : best;
}

/** id を取り除いた並びの index 番目へ入れ直す。 */
export function moveIconInOrder<T>(list: readonly T[], id: T, index: number): T[] {
  const others = list.filter(item => item !== id);
  others.splice(Math.max(0, Math.min(others.length, index)), 0, id);
  return others;
}

/** 全体の並びの中で、見えているボタンが占める場所へ、新しい並び（見えているものだけ）を順に当てはめる。
 *  隠れているボタンは今の場所に残る。 */
export function applyVisibleOrder<T>(full: readonly T[], visibleNew: readonly T[]): T[] {
  const visible = new Set(visibleNew);
  let next = 0;
  return full.map(item => (visible.has(item) ? visibleNew[next++] : item));
}

function positionedElement(id: IconId): HTMLElement | null {
  // Move the template's anchor along with its menu, rather than detaching the
  // toggle from the element that positions the palette.
  return document.getElementById(id === 'prompt-template-toggle' ? 'prompt-template-wrap' : id);
}

export function parseIconPositions(raw: string | null): Positions {
  try {
    const value = JSON.parse(raw || '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    const out: Positions = {};
    for (const id of IDS) {
      const point = value[id];
      if (point && typeof point.x === 'number' && Number.isFinite(point.x) && point.x >= 0
        && typeof point.y === 'number' && Number.isFinite(point.y) && point.y >= 0) {
        out[id] = { x: point.x, y: point.y };
      }
    }
    return out;
  } catch (_) { return {}; }
}

/** Saved sizes are kept within 50%–400%; 100% is the absence of an entry. */
export function parseIconScales(raw: string | null): Scales {
  try {
    const value = JSON.parse(raw || '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    const out: Scales = {};
    for (const id of IDS) {
      const scale = value[id];
      if (typeof scale === 'number' && Number.isFinite(scale) && scale !== 1) {
        out[id] = Math.min(MAX_ICON_SCALE, Math.max(MIN_ICON_SCALE, scale));
      }
    }
    return out;
  } catch (_) { return {}; }
}

/** One mouse notch (deltaY ±100px) changes the size by 10%: up grows, down shrinks.
 *  Smaller trackpad deltas change it proportionally. */
export function nextIconScale(current: number, deltaPx: number): number {
  const next = current * Math.pow(1.1, -deltaPx / 100);
  return Math.round(Math.min(MAX_ICON_SCALE, Math.max(MIN_ICON_SCALE, next)) * 10000) / 10000;
}

/** Leave room for the grip, and keep saved preferences intact when resizing. */
export function clampIconPoint(point: IconPoint, width: number, height: number,
  iconWidth: number, iconHeight: number): IconPoint {
  const maxX = Math.max(0, width - iconWidth - 8);
  const maxY = Math.max(0, height - iconHeight - 8);
  return {
    x: Math.max(Math.min(8, maxX), Math.min(maxX, point.x)),
    y: Math.max(Math.min(8, maxY), Math.min(maxY, point.y)),
  };
}

/** The size an icon renders at. Scaling grows it from its top-left corner. */
function visualSize(id: IconId, icon: HTMLElement): { width: number; height: number } {
  const control = document.getElementById(id) || icon;
  const scale = scales[id] ?? 1;
  return { width: control.offsetWidth * scale, height: control.offsetHeight * scale };
}

/** In the flow (not freely placed), margins absorb the size change so neighbours do not overlap. */
function applyIconScale(control: HTMLElement, scale: number, inFlow: boolean): void {
  if (scale === 1) {
    control.classList.remove('input-icon-scaled', 'input-icon-scale-flow');
    control.style.removeProperty('--input-icon-scale');
    control.style.removeProperty('--input-icon-scale-dx');
    control.style.removeProperty('--input-icon-scale-dy');
    return;
  }
  control.classList.add('input-icon-scaled');
  control.style.setProperty('--input-icon-scale', String(scale));
  control.classList.toggle('input-icon-scale-flow', inFlow);
  if (inFlow) {
    control.style.setProperty('--input-icon-scale-dx', `${(scale - 1) * control.offsetWidth}px`);
    control.style.setProperty('--input-icon-scale-dy', `${(scale - 1) * control.offsetHeight}px`);
  } else {
    control.style.removeProperty('--input-icon-scale-dx');
    control.style.removeProperty('--input-icon-scale-dy');
  }
}

/** The icon a wheel event is over: the control itself, or the grip that belongs to it. */
function iconIdAt(target: EventTarget | null): IconId | null {
  if (!(target instanceof Element)) return null;
  const gripId = target.closest<HTMLElement>('.input-icon-grip')?.dataset.iconId;
  for (const id of IDS) {
    const control = document.getElementById(id);
    if (gripId === id || (control && control.contains(target))) return id;
  }
  return null;
}

function save(): void {
  try { localStorage.setItem(KEY, JSON.stringify(positions)); } catch (_) { /* private mode */ }
}

function saveScales(): void {
  try {
    if (Object.keys(scales).length) localStorage.setItem(SCALE_KEY, JSON.stringify(scales));
    else localStorage.removeItem(SCALE_KEY);
  } catch (_) { /* private mode */ }
}

function saveOrder(): void {
  try {
    if (order.length) localStorage.setItem(ORDER_KEY, JSON.stringify(order));
    else localStorage.removeItem(ORDER_KEY);
  } catch (_) { /* private mode */ }
}

function saveActionsWidth(): void {
  try {
    if (actionsWidth !== null) localStorage.setItem(WIDTH_KEY, String(actionsWidth));
    else localStorage.removeItem(WIDTH_KEY);
  } catch (_) { /* private mode */ }
}

/** 進捗帯が見えていて、ボタンを帯の中へ並べる状態のときの帯。そうでなければ null。 */
function dockedBand(): HTMLElement | null {
  const band = document.getElementById('terminal-live-status');
  return desktopDock() && band && !band.hidden && band.getBoundingClientRect().width > 0 ? band : null;
}

/** 帯の中にボタンを並べているとき、その帯の画面上の矩形。右クリックの一覧を帯全体で受けるために使う。 */
export function getDockedBandRect(): DOMRect | null {
  return dockedBand()?.getBoundingClientRect() ?? null;
}

/** いまの並び（左→右・段の順）。隠れているボタンも含む全体。クイックコマンドの一覧を帯と同じ順に出すために使う。 */
export function getInputIconOrder(): IconId[] {
  return completeIconOrder(order);
}

function effectiveActionsWidth(bandWidth: number): number | undefined {
  return actionsWidth === null ? undefined : clampActionsWidth(actionsWidth, bandWidth);
}

interface VisibleIcon { id: IconId; icon: HTMLElement; width: number; height: number }

/** いま画面に出ている（大きさがある）ボタンを、既定の並びで返す。 */
function collectVisible(dock: boolean): VisibleIcon[] {
  const visible: VisibleIcon[] = [];
  for (const id of IDS) {
    const icon = positionedElement(id);
    if (!icon) continue;
    icon.classList.add('input-icon-movable');
    // Clear fixed placement before measuring newly shown controls. A hidden
    // input-tools parent still has zero dimensions and must stay hidden.
    if (!positions[id] && !dock) icon.classList.remove('input-icon-positioned');
    if (icon.offsetWidth && icon.offsetHeight) {
      visible.push({ id, icon, ...visualSize(id, icon) });
    }
  }
  return visible;
}

/** 見えているボタンを、今の並び（ドラッグ中は下見の並び）に直す。 */
function inCurrentOrder(visible: VisibleIcon[]): VisibleIcon[] {
  const byId = new Map(visible.map(item => [item.id, item] as const));
  const out: VisibleIcon[] = [];
  for (const id of dragOrder ?? completeIconOrder(order)) {
    const item = byId.get(id);
    if (item) out.push(item);
  }
  return out;
}

export function refreshInputIconPositions(): void {
  const wrap = document.getElementById('input-wrap');
  if (!wrap || wrap.clientWidth === 0 || wrap.clientHeight === 0) return;
  const band = document.getElementById('terminal-live-status');
  const dock = dockedBand() !== null;
  const visible = collectVisible(dock);
  const ordered = dock ? inCurrentOrder(visible) : [];
  const bandRect = band?.getBoundingClientRect();
  const layout = dock && bandRect
    ? layoutStatusControls(ordered.map(item => item.width), bandRect.width, ordered.map(item => item.height),
      effectiveActionsWidth(bandRect.width))
    : null;
  if (band) {
    const height = layout?.rows ? Math.max(38, layout.height) : 0;
    band.style.setProperty('--input-actions-height', height ? `${height}px` : '');
    band.style.setProperty('--input-actions-space', layout?.reservedWidth ? `${layout.reservedWidth}px` : '');
  }
  for (const id of IDS) {
    const icon = positionedElement(id);
    if (!icon) continue;
    const item = visible.find(entry => entry.id === id);
    const index = ordered.findIndex(entry => entry.id === id);
    let point: IconPoint | null = null;
    if (dock) {
      if (dragFollow && dragFollow.id === id) point = { x: dragFollow.x, y: dragFollow.y };
      else if (index >= 0 && layout && bandRect) {
        point = { x: bandRect.left + layout.points[index].x, y: bandRect.top + layout.points[index].y };
      }
    } else {
      point = positions[id] || null;
    }
    if (point && item) {
      const applied = clampIconPoint(point, document.documentElement.clientWidth, window.innerHeight,
        item.width, item.height);
      icon.classList.add('input-icon-positioned');
      icon.style.setProperty('--input-icon-x', `${applied.x}px`);
      icon.style.setProperty('--input-icon-y', `${applied.y}px`);
    } else {
      icon.classList.remove('input-icon-positioned');
      icon.style.removeProperty('--input-icon-x');
      icon.style.removeProperty('--input-icon-y');
    }
    const grip = grips.get(id);
    const control = document.getElementById(id);
    if (control) applyIconScale(control, scales[id] ?? 1, !icon.classList.contains('input-icon-positioned'));
    if (grip && control) {
      const rect = control.getBoundingClientRect();
      const gripHidden = rect.width === 0 || rect.height === 0 || getComputedStyle(control).visibility === 'hidden';
      if (grip.hidden !== gripHidden) grip.hidden = gripHidden;
      const gripWidth = grip.offsetWidth || 14;
      const gripHeight = grip.offsetHeight || 14;
      // Centre the grip on the icon's top-right corner: it covers only that corner,
      // so a click anywhere else on the icon stays an ordinary click.
      grip.style.left = `${Math.max(0, Math.min(document.documentElement.clientWidth - gripWidth, rect.right - gripWidth / 2 - 1))}px`;
      grip.style.top = `${Math.max(0, Math.min(window.innerHeight - gripHeight, rect.top - gripHeight / 2))}px`;
      const label = control.getAttribute('aria-label') || control.dataset.tooltip || control.title || control.textContent || id;
      grip.setAttribute('aria-label', `↔ ↕ ${label.trim()}`);
    }
  }
  if (resizer) {
    const show = dock && !!bandRect && !!layout && layout.rows > 0;
    if (resizer.hidden === show) resizer.hidden = !show;
    if (show && bandRect && layout) {
      lastReserved = layout.reservedWidth;
      resizer.style.height = `${Math.max(16, bandRect.height - 8)}px`;
      // 帯の右の余白（10px）とボタン領域の幅のぶんだけ右から戻った所が、メッセージとの境目。
      resizer.style.left = `${Math.max(0, bandRect.right - 10 - layout.reservedWidth - 5)}px`;
      resizer.style.top = `${bandRect.top + 4}px`;
    }
  }
}

export function resetInputIconPositions(): void {
  cancelDrag?.();
  positions = {};
  order = [];
  dragOrder = null;
  dragFollow = null;
  actionsWidth = null;
  try {
    localStorage.removeItem(KEY);
    localStorage.removeItem(OLD_KEY);
    localStorage.removeItem(ORDER_KEY);
    localStorage.removeItem(WIDTH_KEY);
    localStorage.removeItem('inputPrimaryToolsOffset');
  } catch (_) { /* private mode */ }
  refreshInputIconPositions();
}

export function resetInputIconScales(): void {
  scales = {};
  saveScales();
  refreshInputIconPositions();
}

/** 帯の中で、見えているボタンの並び（左→右・段の順）。 */
function visibleOrderIds(): IconId[] {
  return inCurrentOrder(collectVisible(true)).map(item => item.id);
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(Math.max(min, max), value));
}

export function initInputIconPositions(): void {
  const wrap = document.getElementById('input-wrap');
  if (!wrap) return;
  try {
    scales = parseIconScales(localStorage.getItem(SCALE_KEY));
    const saved = localStorage.getItem(KEY);
    positions = parseIconPositions(saved);
    if (saved === null) {
      const old = parseIconPositions(localStorage.getItem(OLD_KEY));
      const rect = wrap.getBoundingClientRect();
      for (const id of IDS) {
        if (old[id]) positions[id] = { x: rect.left + old[id].x, y: rect.top + old[id].y };
      }
      if (Object.keys(positions).length) save();
    }
    const savedOrder = localStorage.getItem(ORDER_KEY);
    order = parseIconOrder(savedOrder);
    // 初めて帯の並びを使うときは、これまでの座標を並びへ読み替える（座標は帯が無い画面のために残す）。
    if (savedOrder === null) {
      order = orderFromPositions(positions);
      if (order.length) saveOrder();
    }
    actionsWidth = parseActionsWidth(localStorage.getItem(WIDTH_KEY));
  } catch (_) { /* private mode */ }

  for (const id of IDS) {
    const control = document.getElementById(id);
    const icon = positionedElement(id);
    if (!icon || !control) continue;
    control.setAttribute('aria-keyshortcuts', 'Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown');
    const grip = document.createElement('button');
    grip.type = 'button';
    grip.className = 'ui-button ui-button--quiet ui-button--compact input-icon-grip';
    grip.textContent = '⠿';
    grip.title = '↔ ↕ · Alt + ← ↑ → ↓ · Ctrl/⌘ + 🖱 50–400%';
    grip.dataset.iconId = id;
    grip.hidden = true;
    grips.set(id, grip);
    wrap.append(grip);
    // Stop the gesture before it reaches send's IME mousedown handler or the
    // voice click handler. Other clicks on the icon behave exactly as before.
    for (const event of ['mousedown', 'click', 'dblclick']) {
      grip.addEventListener(event, e => { e.preventDefault(); e.stopPropagation(); });
    }
    grip.addEventListener('pointerdown', e => {
      if (e.button !== 0 || !e.isPrimary || cancelDrag) return;
      e.preventDefault();
      e.stopPropagation();
      const band = dockedBand();
      const previous = positions[id];
      const rect = icon.getBoundingClientRect();
      const grabX = e.clientX - rect.left;
      const grabY = e.clientY - rect.top;
      const pointerId = e.pointerId;
      const orderBefore = completeIconOrder(order);
      grip.setPointerCapture(pointerId);
      icon.classList.add('input-icon-moving');

      const move = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        ev.preventDefault();
        const size = visualSize(id, icon);
        if (band) {
          // 帯の中だけで動く。ほかのボタンの並びで落とし先を決め、下見の並びへ組み替える。
          const b = band.getBoundingClientRect();
          dragFollow = {
            id,
            x: clamp(ev.clientX - grabX, b.left, b.right - size.width),
            y: clamp(ev.clientY - grabY, b.top, b.bottom - size.height),
          };
          const others = inCurrentOrderFrom(orderBefore).filter(item => item.id !== id);
          const lay = layoutStatusControls(others.map(item => item.width), b.width,
            others.map(item => item.height), effectiveActionsWidth(b.width));
          const slots = lay.points.map((p, i) => ({
            cx: b.left + p.x + others[i].width / 2,
            cy: b.top + p.y + others[i].height / 2,
          }));
          const at = insertionIndex(slots, ev.clientX, ev.clientY);
          dragOrder = applyVisibleOrder(orderBefore, moveIconInOrder(others.map(item => item.id), id, at));
        } else {
          positions[id] = clampIconPoint({
            x: ev.clientX - grabX,
            y: ev.clientY - grabY,
          }, document.documentElement.clientWidth, window.innerHeight, size.width, size.height);
        }
        refreshInputIconPositions();
      };
      const finish = (commit: boolean) => {
        grip.removeEventListener('pointermove', move);
        grip.removeEventListener('pointerup', up);
        grip.removeEventListener('pointercancel', cancel);
        grip.removeEventListener('lostpointercapture', cancel);
        document.removeEventListener('keydown', escape, true);
        cancelDrag = null;
        if (grip.hasPointerCapture(pointerId)) grip.releasePointerCapture(pointerId);
        icon.classList.remove('input-icon-moving');
        if (band) {
          if (commit && dragOrder) { order = dragOrder; saveOrder(); }
          dragOrder = null;
          dragFollow = null;
          refreshInputIconPositions();
        } else if (commit) save();
        else {
          if (previous) positions[id] = previous;
          else delete positions[id];
          refreshInputIconPositions();
        }
      };
      const up = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        move(ev);
        finish(true);
      };
      const cancel = () => finish(false);
      const escape = (ev: KeyboardEvent) => {
        if (ev.key !== 'Escape') return;
        ev.preventDefault();
        ev.stopPropagation();
        cancel();
      };
      cancelDrag = cancel;
      grip.addEventListener('pointermove', move);
      grip.addEventListener('pointerup', up);
      grip.addEventListener('pointercancel', cancel);
      grip.addEventListener('lostpointercapture', cancel);
      document.addEventListener('keydown', escape, true);
    });
    const keyboardMove = (e: KeyboardEvent) => {
      if (cancelDrag || (e.currentTarget !== grip && !e.altKey)
        || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
      e.preventDefault();
      e.stopPropagation();
      if (dockedBand()) {
        // 帯の中では左右で並びを 1 つずつ動かす（上下は使わない）。
        if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
        const ids = visibleOrderIds();
        const at = ids.indexOf(id);
        if (at < 0) return;
        const target = e.key === 'ArrowLeft' ? at - 1 : at + 1;
        if (target < 0 || target > ids.length - 1) return;
        order = applyVisibleOrder(completeIconOrder(order), moveIconInOrder(ids, id, target));
        saveOrder();
        refreshInputIconPositions();
        return;
      }
      const rect = icon.getBoundingClientRect();
      const point = positions[id] || {
        x: rect.left,
        y: rect.top,
      };
      const step = e.shiftKey ? 20 : 4;
      const size = visualSize(id, icon);
      positions[id] = clampIconPoint({
        x: point.x + (e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0),
        y: point.y + (e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0),
      }, document.documentElement.clientWidth, window.innerHeight, size.width, size.height);
      refreshInputIconPositions();
      save();
    };
    control.addEventListener('keydown', keyboardMove);
    grip.addEventListener('keydown', keyboardMove);
  }

  // 帯の中のメッセージ（左）とボタン群（右）の境目。ドラッグでボタン領域の幅を変え、
  // ダブルクリックで自動の幅へ戻す。キーボードは ← で広げ、→ で狭める。
  resizer = document.createElement('button');
  resizer.type = 'button';
  resizer.className = 'ui-button ui-button--quiet ui-button--compact input-actions-resizer';
  resizer.textContent = '⋮';
  resizer.title = '↔ · ← → · ×2 auto';
  resizer.setAttribute('aria-label', 'Resize status and buttons');
  resizer.hidden = true;
  wrap.append(resizer);
  const handle = resizer;
  for (const event of ['mousedown', 'click']) {
    handle.addEventListener(event, e => { e.preventDefault(); e.stopPropagation(); });
  }
  handle.addEventListener('dblclick', e => {
    e.preventDefault();
    e.stopPropagation();
    actionsWidth = null;
    saveActionsWidth();
    refreshInputIconPositions();
  });
  handle.addEventListener('pointerdown', e => {
    const band = dockedBand();
    if (e.button !== 0 || !e.isPrimary || cancelDrag || !band) return;
    e.preventDefault();
    e.stopPropagation();
    const pointerId = e.pointerId;
    const before = actionsWidth;
    handle.setPointerCapture(pointerId);
    handle.classList.add('is-dragging');
    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      ev.preventDefault();
      const b = band.getBoundingClientRect();
      // 境目の右側が、帯の右の余白（10px）を除いたボタン領域になる。
      actionsWidth = clampActionsWidth(b.right - 10 - ev.clientX, b.width);
      refreshInputIconPositions();
    };
    const finish = (commit: boolean) => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', cancel);
      handle.removeEventListener('lostpointercapture', cancel);
      document.removeEventListener('keydown', escape, true);
      cancelDrag = null;
      if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
      handle.classList.remove('is-dragging');
      if (commit) saveActionsWidth();
      else actionsWidth = before;
      refreshInputIconPositions();
    };
    const up = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      move(ev);
      finish(true);
    };
    const cancel = () => finish(false);
    const escape = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape') return;
      ev.preventDefault();
      ev.stopPropagation();
      cancel();
    };
    cancelDrag = cancel;
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', cancel);
    handle.addEventListener('lostpointercapture', cancel);
    document.addEventListener('keydown', escape, true);
  });
  handle.addEventListener('keydown', e => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    const band = dockedBand();
    if (!band) return;
    e.preventDefault();
    e.stopPropagation();
    const current = actionsWidth ?? lastReserved;
    const next = current + (e.key === 'ArrowLeft' ? 16 : -16);
    actionsWidth = clampActionsWidth(next, band.getBoundingClientRect().width);
    saveActionsWidth();
    refreshInputIconPositions();
  });

  // Ctrl+wheel (⌘+wheel on macOS) over an icon or its grip resizes that icon.
  // Window capture runs before the terminal's document-level capture handler,
  // and preventDefault keeps the browser from zooming the whole page instead.
  window.addEventListener('wheel', (e: WheelEvent) => {
    if (!e.ctrlKey && !e.metaKey) return;
    const id = iconIdAt(e.target);
    if (!id) return;
    e.preventDefault();
    e.stopPropagation();
    if (cancelDrag || !e.deltaY) return;
    const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 33
      : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? 400 : 1;
    const next = nextIconScale(scales[id] ?? 1, e.deltaY * unit);
    if (next === 1) delete scales[id];
    else scales[id] = next;
    saveScales();
    refreshInputIconPositions();
  }, { passive: false, capture: true });
  let scheduled = false;
  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => { scheduled = false; refreshInputIconPositions(); });
  };
  const resize = new ResizeObserver(schedule);
  resize.observe(wrap);
  const band = document.getElementById('terminal-live-status');
  if (band) resize.observe(band);
  IDS.forEach(id => { const el = positionedElement(id); if (el) resize.observe(el); });
  // Observe label replacement, control visibility, and side-switch DOM moves.
  // Do not observe style/class: refresh itself changes both.
  new MutationObserver(schedule).observe(wrap, {
    subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['hidden'],
  });
  wrap.addEventListener('scroll', schedule, true);
  window.addEventListener('resize', schedule);
  window.addEventListener('scroll', schedule, true);
  refreshInputIconPositions();
}

/** 全体の並び（ドラッグを始めた時点のもの）で、見えているボタンを並べる。ドラッグ中の下見の影響を受けない。 */
function inCurrentOrderFrom(fullOrder: readonly IconId[]): VisibleIcon[] {
  const byId = new Map(collectVisible(true).map(item => [item.id, item] as const));
  const out: VisibleIcon[] = [];
  for (const id of fullOrder) {
    const item = byId.get(id);
    if (item) out.push(item);
  }
  return out;
}

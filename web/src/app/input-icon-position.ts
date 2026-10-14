// Independent pointer placement and size. Only the small grip on an icon's
// top-right corner moves it; the icon's existing handlers remain responsible for
// ordinary clicks. Grips are sibling buttons, so changing command labels does not
// remove them or their listeners. Ctrl+wheel (⌘+wheel on macOS) over an icon or
// its grip resizes that icon alone, from 50% to 400%.
const KEY = 'inputIconPositionsV2';
const OLD_KEY = 'inputIconPositionsV1';
const SCALE_KEY = 'inputIconScalesV1';
export const MIN_ICON_SCALE = 0.5;
export const MAX_ICON_SCALE = 4;
export const INPUT_ICON_IDS = [
  'mobile-composer-attach-btn', 'mobile-keyboard-toggle',
  'input-clear-btn', 'send-btn', 'voice-btn', 'prompt-template-toggle',
  'voice-wakeword-btn', 'quick-clear-btn', 'quick-model-btn',
  'quick-cmd-btn-3', 'quick-cmd-btn-4', 'quick-cmd-btn-5',
  'slash-picker-btn', 'buf-clear-btn',
] as const;
const IDS = INPUT_ICON_IDS;
type IconId = typeof IDS[number];
export interface IconPoint { x: number; y: number }
type Positions = Partial<Record<IconId, IconPoint>>;
type Scales = Partial<Record<IconId, number>>;
let positions: Positions = {};
let scales: Scales = {};
const grips = new Map<IconId, HTMLButtonElement>();
let cancelDrag: (() => void) | null = null;
const desktopDock = () => window.matchMedia('(min-width: 721px)').matches;

/** Lay out the default controls at the right of the status band, leaving its text readable.
 *  A row is as tall as its tallest (resized) control, and never shorter than an unscaled row. */
export function layoutStatusControls(widths: number[], bandWidth: number, heights: number[] = []): {
  points: IconPoint[]; rows: number; reservedWidth: number; height: number;
} {
  const gap = 6;
  const available = Math.max(1, bandWidth - 180);
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
  return { points, rows: rows.length, reservedWidth: (rows[0]?.width ?? 0) + 18, height: rows.length ? y - 1 : 0 };
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

export function refreshInputIconPositions(): void {
  const wrap = document.getElementById('input-wrap');
  if (!wrap || wrap.clientWidth === 0 || wrap.clientHeight === 0) return;
  const band = document.getElementById('terminal-live-status');
  const dock = desktopDock() && band && !band.hidden && band.getBoundingClientRect().width > 0;
  const visible: { id: IconId; icon: HTMLElement; width: number; height: number }[] = [];
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
  const defaults = dock ? visible.filter(item => !positions[item.id]) : [];
  const bandRect = band?.getBoundingClientRect();
  const layout = bandRect
    ? layoutStatusControls(defaults.map(item => item.width), bandRect.width, defaults.map(item => item.height))
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
    const index = defaults.findIndex(entry => entry.id === id);
    const custom = positions[id];
    const point = custom || (index >= 0 && layout && bandRect
      ? { x: bandRect.left + layout.points[index].x, y: bandRect.top + layout.points[index].y }
      : null);
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
}

export function resetInputIconPositions(): void {
  cancelDrag?.();
  positions = {};
  try {
    localStorage.removeItem(KEY);
    localStorage.removeItem(OLD_KEY);
    localStorage.removeItem('inputPrimaryToolsOffset');
  } catch (_) { /* private mode */ }
  refreshInputIconPositions();
}

export function resetInputIconScales(): void {
  scales = {};
  saveScales();
  refreshInputIconPositions();
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
      const previous = positions[id];
      const rect = icon.getBoundingClientRect();
      const grabX = e.clientX - rect.left;
      const grabY = e.clientY - rect.top;
      const pointerId = e.pointerId;
      grip.setPointerCapture(pointerId);
      icon.classList.add('input-icon-moving');

      const move = (ev: PointerEvent) => {
        if (ev.pointerId !== pointerId) return;
        ev.preventDefault();
        const size = visualSize(id, icon);
        positions[id] = clampIconPoint({
          x: ev.clientX - grabX,
          y: ev.clientY - grabY,
        }, document.documentElement.clientWidth, window.innerHeight, size.width, size.height);
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
        if (commit) save();
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

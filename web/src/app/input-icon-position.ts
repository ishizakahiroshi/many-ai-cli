// Independent pointer placement. Only the grip moves an icon; the icon's
// existing handlers remain responsible for ordinary clicks. Grips are sibling
// buttons, so changing command labels does not remove them or their listeners.
const KEY = 'inputIconPositionsV2';
const OLD_KEY = 'inputIconPositionsV1';
export const INPUT_ICON_IDS = [
  'tools-flip-btn', 'mobile-composer-attach-btn', 'mobile-keyboard-toggle',
  'input-clear-btn', 'send-btn', 'voice-btn', 'prompt-template-toggle',
  'voice-wakeword-btn', 'quick-clear-btn', 'quick-model-btn',
  'quick-cmd-btn-3', 'quick-cmd-btn-4', 'quick-cmd-btn-5',
  'slash-picker-btn', 'buf-clear-btn',
] as const;
const IDS = INPUT_ICON_IDS;
type IconId = typeof IDS[number];
export interface IconPoint { x: number; y: number }
type Positions = Partial<Record<IconId, IconPoint>>;
let positions: Positions = {};
const grips = new Map<IconId, HTMLButtonElement>();
let cancelDrag: (() => void) | null = null;
const desktopDock = () => window.matchMedia('(min-width: 721px)').matches;

/** Lay out the default controls at the right of the status band, leaving its text readable. */
export function layoutStatusControls(widths: number[], bandWidth: number): {
  points: IconPoint[]; rows: number; reservedWidth: number;
} {
  const gap = 6;
  const available = Math.max(1, bandWidth - 180);
  const points: IconPoint[] = [];
  const rows: { start: number; width: number }[] = [];
  for (let i = 0; i < widths.length; i++) {
    let row = rows[rows.length - 1];
    if (!row || row.width + gap + widths[i] > available) {
      row = { start: i, width: 0 };
      rows.push(row);
    }
    row.width += (i === row.start ? 0 : gap) + widths[i];
  }
  rows.forEach((row, rowIndex) => {
    let x = bandWidth - 8 - row.width;
    const end = rows[rowIndex + 1]?.start ?? widths.length;
    for (let i = row.start; i < end; i++) {
      points[i] = { x, y: 5 + rowIndex * 38 };
      x += widths[i] + gap;
    }
  });
  return { points, rows: rows.length, reservedWidth: (rows[0]?.width ?? 0) + 18 };
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

function save(): void {
  try { localStorage.setItem(KEY, JSON.stringify(positions)); } catch (_) { /* private mode */ }
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
      visible.push({ id, icon, width: icon.offsetWidth, height: icon.offsetHeight });
    }
  }
  const defaults = dock ? visible.filter(item => !positions[item.id]) : [];
  const bandRect = band?.getBoundingClientRect();
  const layout = bandRect ? layoutStatusControls(defaults.map(item => item.width), bandRect.width) : null;
  if (band) {
    const height = layout?.rows ? Math.max(38, layout.rows * 38 + 4) : 0;
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
    if (grip && control) {
      const rect = control.getBoundingClientRect();
      const gripHidden = rect.width === 0 || rect.height === 0 || getComputedStyle(control).visibility === 'hidden';
      if (grip.hidden !== gripHidden) grip.hidden = gripHidden;
      const gripWidth = grip.offsetWidth || 18;
      const gripHeight = grip.offsetHeight || 18;
      grip.style.left = `${Math.max(0, Math.min(document.documentElement.clientWidth - gripWidth, rect.right - gripWidth + 5))}px`;
      grip.style.top = `${Math.max(0, Math.min(window.innerHeight - gripHeight, rect.top - 6))}px`;
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

export function initInputIconPositions(): void {
  const wrap = document.getElementById('input-wrap');
  if (!wrap) return;
  try {
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
    grip.title = '↔ ↕ · Alt + ← ↑ → ↓';
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
        positions[id] = clampIconPoint({
          x: ev.clientX - grabX,
          y: ev.clientY - grabY,
        }, document.documentElement.clientWidth, window.innerHeight, icon.offsetWidth, icon.offsetHeight);
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
      positions[id] = clampIconPoint({
        x: point.x + (e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0),
        y: point.y + (e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0),
      }, document.documentElement.clientWidth, window.innerHeight, icon.offsetWidth, icon.offsetHeight);
      refreshInputIconPositions();
      save();
    };
    control.addEventListener('keydown', keyboardMove);
    grip.addEventListener('keydown', keyboardMove);
  }
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

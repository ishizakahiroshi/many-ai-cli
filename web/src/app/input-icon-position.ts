// Independent pointer placement. Only the grip moves an icon; the icon's
// existing handlers remain responsible for ordinary clicks. Grips are sibling
// buttons, so changing command labels does not remove them or their listeners.
const KEY = 'inputIconPositionsV1';
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
const slots = new Map<IconId, HTMLElement>();
const grips = new Map<IconId, HTMLButtonElement>();
let cancelDrag: (() => void) | null = null;

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
  for (const id of IDS) {
    const icon = positionedElement(id);
    if (!icon) continue;
    icon.classList.add('input-icon-movable');
    const point = positions[id];
    if (!point) {
      icon.classList.remove('input-icon-positioned');
      icon.style.removeProperty('--input-icon-x');
      icon.style.removeProperty('--input-icon-y');
      slots.get(id)?.remove();
      slots.delete(id);
    } else {
      let slot = slots.get(id);
      if (!slot) {
        slot = document.createElement('span');
        slot.className = 'input-icon-slot';
        slot.setAttribute('aria-hidden', 'true');
        slots.set(id, slot);
      }
      // Keep the former flex/grid footprint, including after the side switch
      // reorders DOM nodes. Moving icons must not collapse the input box.
      if (slot.nextSibling !== icon) icon.parentElement?.insertBefore(slot, icon);
      const slotHidden = icon.offsetWidth === 0 || icon.offsetHeight === 0;
      if (slot.hidden !== slotHidden) slot.hidden = slotHidden;
      slot.style.width = `${icon.offsetWidth}px`;
      slot.style.height = `${icon.offsetHeight}px`;
      slot.style.setProperty('--tool-order', icon.style.getPropertyValue('--tool-order') || '0');
      icon.classList.add('input-icon-positioned');
      const applied = clampIconPoint(point, wrap.clientWidth, wrap.clientHeight, icon.offsetWidth, icon.offsetHeight);
      icon.style.setProperty('--input-icon-x', `${applied.x}px`);
      icon.style.setProperty('--input-icon-y', `${applied.y}px`);
    }
    const grip = grips.get(id);
    const control = document.getElementById(id);
    if (grip && control) {
      const rect = control.getBoundingClientRect();
      const bounds = wrap.getBoundingClientRect();
      const gripHidden = rect.width === 0 || rect.height === 0 || getComputedStyle(control).visibility === 'hidden';
      if (grip.hidden !== gripHidden) grip.hidden = gripHidden;
      const scaleX = bounds.width / wrap.offsetWidth || 1;
      const scaleY = bounds.height / wrap.offsetHeight || 1;
      const x = (rect.right - bounds.left) / scaleX - wrap.clientLeft - 13;
      const y = (rect.top - bounds.top) / scaleY - wrap.clientTop - 6;
      grip.style.left = `${Math.max(0, Math.min(wrap.clientWidth - 18, x))}px`;
      grip.style.top = `${Math.max(0, Math.min(wrap.clientHeight - 18, y))}px`;
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
    localStorage.removeItem('inputPrimaryToolsOffset');
  } catch (_) { /* private mode */ }
  refreshInputIconPositions();
}

export function initInputIconPositions(): void {
  const wrap = document.getElementById('input-wrap');
  if (!wrap) return;
  try { positions = parseIconPositions(localStorage.getItem(KEY)); } catch (_) { /* private mode */ }

  for (const id of IDS) {
    const control = document.getElementById(id);
    const icon = positionedElement(id);
    if (!icon || !control) continue;
    control.setAttribute('aria-keyshortcuts', 'Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown');
    const grip = document.createElement('button');
    grip.type = 'button';
    grip.className = 'input-icon-grip';
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
        const bounds = wrap.getBoundingClientRect();
        // Pointer coordinates are viewport CSS pixels; account for transformed
        // input boxes before storing local CSS coordinates.
        const scaleX = bounds.width / wrap.offsetWidth || 1;
        const scaleY = bounds.height / wrap.offsetHeight || 1;
        positions[id] = clampIconPoint({
          x: (ev.clientX - bounds.left - grabX) / scaleX - wrap.clientLeft,
          y: (ev.clientY - bounds.top - grabY) / scaleY - wrap.clientTop,
        }, wrap.clientWidth, wrap.clientHeight, icon.offsetWidth, icon.offsetHeight);
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
      const bounds = wrap.getBoundingClientRect();
      const rect = icon.getBoundingClientRect();
      const point = positions[id] || {
        x: (rect.left - bounds.left) / (bounds.width / wrap.offsetWidth || 1) - wrap.clientLeft,
        y: (rect.top - bounds.top) / (bounds.height / wrap.offsetHeight || 1) - wrap.clientTop,
      };
      const step = e.shiftKey ? 20 : 4;
      positions[id] = clampIconPoint({
        x: point.x + (e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0),
        y: point.y + (e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0),
      }, wrap.clientWidth, wrap.clientHeight, icon.offsetWidth, icon.offsetHeight);
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
  IDS.forEach(id => { const el = positionedElement(id); if (el) resize.observe(el); });
  // Observe label replacement, control visibility, and side-switch DOM moves.
  // Do not observe style/class: refresh itself changes both.
  new MutationObserver(schedule).observe(wrap, {
    subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['hidden'],
  });
  wrap.addEventListener('scroll', schedule, true);
  refreshInputIconPositions();
}

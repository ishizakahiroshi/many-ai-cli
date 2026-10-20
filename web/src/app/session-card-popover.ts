// Compact-card details are an overlay, never a child that changes row height.
const SHOW_DELAY_MS = 400;
const EDGE_MARGIN = 6;
let pop: HTMLDivElement | null = null;
let anchor: HTMLElement | null = null;
let details: (() => string) | null = null;
let showTimer = 0;
let listening = false;

export function hideSessionCardPopover(): void {
  window.clearTimeout(showTimer);
  showTimer = 0;
  anchor?.removeAttribute('aria-describedby');
  anchor = null;
  details = null;
  if (pop) pop.hidden = true;
}

function listenForDismissal(): void {
  if (listening) return;
  listening = true;
  // Cancel pending timers too, not only an already visible popover.
  document.addEventListener('scroll', hideSessionCardPopover, true);
  document.addEventListener('pointerdown', hideSessionCardPopover, true);
  document.addEventListener('dragstart', hideSessionCardPopover, true);
  document.addEventListener('visibilitychange', hideSessionCardPopover);
  document.addEventListener('keydown', e => { if (e.key === 'Escape') hideSessionCardPopover(); });
  window.addEventListener('blur', hideSessionCardPopover);
  window.addEventListener('resize', hideSessionCardPopover);
}

function show(): void {
  showTimer = 0;
  if (!anchor?.isConnected || !details) { hideSessionCardPopover(); return; }
  if (!pop) {
    pop = document.createElement('div');
    pop.id = 'session-card-popover';
    pop.className = 'session-card-popover';
    pop.setAttribute('role', 'tooltip');
    pop.dataset.wheelNative = '';
    document.body.appendChild(pop);
  }
  const html = details();
  if (pop.innerHTML !== html) pop.innerHTML = html;
  pop.hidden = false;
  anchor.setAttribute('aria-describedby', pop.id);
  const r = anchor.getBoundingClientRect();
  const left = Math.max(EDGE_MARGIN, Math.min(r.left, window.innerWidth - pop.offsetWidth - EDGE_MARGIN));
  let top = r.bottom + 2;
  if (top + pop.offsetHeight > window.innerHeight - EDGE_MARGIN) top = Math.max(EDGE_MARGIN, r.top - pop.offsetHeight - 2);
  pop.style.left = `${Math.round(left)}px`;
  pop.style.top = `${Math.round(top)}px`;
}

/** Live data changes replace visible contents immediately, without reopening a dismissed popover. */
export function refreshSessionCardPopover(card: HTMLElement): void {
  if (anchor === card && pop && !pop.hidden) show();
}

export function attachSessionCardPopover(card: HTMLElement, getDetails: () => string): void {
  listenForDismissal();
  const canHover = () => window.matchMedia('(hover: hover)').matches;
  const schedule = () => {
    const wasOpen = !!pop && !pop.hidden;
    hideSessionCardPopover();
    anchor = card;
    details = getDetails;
    showTimer = window.setTimeout(show, wasOpen ? 0 : SHOW_DELAY_MS);
  };
  card.addEventListener('pointerenter', e => {
    if (e.pointerType === 'mouse' && canHover()) schedule();
  });
  card.addEventListener('pointermove', e => {
    // Merely sweeping across a row must not start showing its details.
    if (e.pointerType === 'mouse' && anchor === card && showTimer) schedule();
  });
  card.addEventListener('pointerleave', () => { if (anchor === card) hideSessionCardPopover(); });
  card.addEventListener('focusin', e => {
    if (canHover() && (e.target as HTMLElement).matches(':focus-visible')) schedule();
  });
  card.addEventListener('focusout', e => {
    if (!card.contains(e.relatedTarget as Node) && anchor === card) hideSessionCardPopover();
  });
}

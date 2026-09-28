// Move a pane by keyboard and provide compact screen switching.
// Touch placement requests commit immediately through the layout owner.

type PlacementRequest = {
  kind: 'tab' | 'session' | 'move';
  source?: string;
  tabName?: string;
  sessionId?: number | null;
  fromIdx?: number;
  anchor?: HTMLElement;
};

const placeableTabs = new Set([
  'terminal', 'chat', 'files', 'git', 'review', 'approval', 'history', 'orchestration',
]);

let placementOverlay: HTMLElement | null = null;
let returnFocus: HTMLElement | null = null;

function label(key: string, fallback: string): string {
  const translated = (window as any).t?.(key);
  return translated && translated !== key ? translated : fallback;
}

function manager(): any {
  return (window as any).multiPaneManager;
}

function slotTitle(slot: any, idx: number): string {
  if (!slot) return label('pane_placement_empty', `空き ${idx + 1}`);
  const session = slot.session;
  if (session) {
    const name = session.label || session.cwd || '';
    return `#${session.id}${name ? ` · ${name}` : ''}`;
  }
  const tab = slot.tab;
  if (tab) {
    const name = tab.tabName || tab.name || '';
    const sid = tab.sessionId == null ? '' : `#${tab.sessionId} · `;
    return `${sid}${name || label('pane_placement_view', '画面')}`;
  }
  return label('pane_placement_empty', `空き ${idx + 1}`);
}

function existingSlotIndex(mgr: any, request: PlacementRequest): number {
  if (request.kind === 'move') return request.fromIdx ?? -1;
  return (mgr.slots || []).findIndex((slot: any) => {
    if (request.kind === 'session') return slot?.session?.id === request.sessionId;
    return slot?.tab?.tabName === request.tabName && slot?.tab?.sessionId === (request.sessionId ?? null);
  });
}

function validRequest(raw: unknown): PlacementRequest | null {
  if (!raw || typeof raw !== 'object') return null;
  const request = raw as PlacementRequest;
  if (request.kind === 'tab') {
    if (!request.tabName || !placeableTabs.has(request.tabName)) return null;
    const globalTab = ['approval', 'history', 'orchestration'].includes(request.tabName);
    if (!globalTab && !Number.isFinite(request.sessionId)) return null;
    if (request.sessionId != null && !Number.isFinite(request.sessionId)) return null;
  } else if (request.kind === 'session') {
    if (!Number.isFinite(request.sessionId)) return null;
  } else if (request.kind === 'move') {
    if (!Number.isInteger(request.fromIdx) || (request.fromIdx as number) < 0) return null;
  } else return null;
  return request;
}

function closePlacementMenu(): void {
  if (!placementOverlay) return;
  placementOverlay.remove();
  placementOverlay = null;
  document.removeEventListener('keydown', onPlacementKeyDown, true);
  const focus = returnFocus;
  returnFocus = null;
  if (focus?.isConnected) focus.focus();
}

function onPlacementKeyDown(event: KeyboardEvent): void {
  if (!placementOverlay) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    closePlacementMenu();
    return;
  }
  if (event.key !== 'Tab') return;
  const controls = Array.from(placementOverlay.querySelectorAll<HTMLElement>('button:not([disabled])'));
  if (controls.length === 0) return;
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}

function commitPlacement(request: PlacementRequest, targetIdx?: number): void {
  closePlacementMenu();
  (window as any).closeMobileSessionDrawer?.();
  window.dispatchEvent(new CustomEvent('pane-placement-commit', {
    detail: {
      kind: request.kind,
      tabName: request.tabName,
      sessionId: request.sessionId,
      fromIdx: request.fromIdx,
      targetIdx,
    },
  }));
}

function openPlacementMenu(request: PlacementRequest): void {
  const mgr = manager();
  if (!mgr) return;
  closePlacementMenu();
  returnFocus = request.anchor instanceof HTMLElement ? request.anchor : document.activeElement as HTMLElement;

  const overlay = document.createElement('div');
  overlay.className = 'pane-placement-overlay aac-wheel-overlay';
  overlay.addEventListener('pointerdown', event => {
    if (event.target === overlay) closePlacementMenu();
  });
  const dialog = document.createElement('section');
  dialog.className = 'pane-placement-dialog';
  dialog.setAttribute('role', 'dialog');
  dialog.setAttribute('aria-modal', 'true');
  dialog.setAttribute('aria-labelledby', 'pane-placement-heading');
  const heading = document.createElement('h2');
  heading.id = 'pane-placement-heading';
  heading.textContent = label('pane_placement_move_to', '移動先を選ぶ');
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'pane-placement-cancel';
  close.textContent = label('pane_placement_cancel', '閉じる');
  close.addEventListener('click', closePlacementMenu);
  const header = document.createElement('div');
  header.className = 'pane-placement-dialog-header';
  header.append(heading, close);
  dialog.appendChild(header);

  const choices = document.createElement('div');
  choices.className = 'pane-placement-choices';
  const capacity = Math.max(0, Number(mgr.cols) * Number(mgr.rows));
  const sourceIdx = existingSlotIndex(mgr, request);
  for (let idx = 0; idx < capacity; idx++) {
    if (idx === sourceIdx) continue;
    const slot = mgr.slots?.[idx] || null;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'pane-placement-choice';
    const action = slot ? label('pane_placement_swap', '入れ替え') : label('pane_placement_move', '移動');
    button.textContent = `${idx + 1}. ${slotTitle(slot, idx)} · ${action}`;
    button.addEventListener('click', () => commitPlacement(request, idx));
    choices.appendChild(button);
  }
  if (choices.childElementCount === 0) {
    const hint = document.createElement('p');
    hint.className = 'pane-placement-empty';
    hint.textContent = label('pane_placement_no_target', '移動先の枠がありません。マルチのレイアウトを増やしてください。');
    choices.appendChild(hint);
  }
  dialog.appendChild(choices);
  overlay.appendChild(dialog);
  document.body.appendChild(overlay);
  placementOverlay = overlay;
  document.addEventListener('keydown', onPlacementKeyDown, true);
  const initial = choices.querySelector('button') as HTMLButtonElement | null;
  (initial || close).focus();
}

function renderCompactPaneSwitcher(): void {
  const switcher = document.getElementById('compact-pane-switcher');
  const select = document.getElementById('compact-pane-select') as HTMLSelectElement | null;
  const multiView = document.getElementById('multi-view');
  const mgr = manager();
  if (!switcher || !select || !multiView || !mgr) return;
  const compact = window.matchMedia('(max-width: 1000px)').matches;
  switcher.hidden = !compact || multiView.hidden;
  if (switcher.hidden) return;

  const capacity = Math.max(0, Number(mgr.cols) * Number(mgr.rows));
  const active = Math.max(0, Math.min(capacity - 1, Number(mgr.getMobileActiveSlot?.() ?? 0)));
  select.replaceChildren();
  for (let idx = 0; idx < capacity; idx++) {
    const option = document.createElement('option');
    option.value = String(idx);
    option.textContent = `${idx + 1}. ${slotTitle(mgr.slots?.[idx], idx)}`;
    select.appendChild(option);
  }
  select.value = String(active);
  const scope = mgr.getScopeStatus?.();
  switcher.querySelectorAll<HTMLButtonElement>('[data-pane-scope]').forEach(button => {
    const value = button.dataset.paneScope;
    button.setAttribute('aria-pressed', String(scope?.effective === value));
    if (value === 'box') button.disabled = !scope?.projectKey;
  });
}

function initPanePlacementMenu(): void {
  window.addEventListener('pane-placement-request', event => {
    const request = validRequest((event as CustomEvent).detail);
    if (!request || request.source === 'display-area') return;
    if (request.kind === 'move') openPlacementMenu(request);
    else commitPlacement(request);
  });

  const stack = document.getElementById('display-stack');
  const multiView = document.getElementById('multi-view');
  if (!stack || !multiView) return;
  const switcher = document.createElement('div');
  switcher.id = 'compact-pane-switcher';
  switcher.hidden = true;
  const scope = document.createElement('div');
  scope.className = 'compact-pane-scope';
  scope.setAttribute('role', 'group');
  scope.setAttribute('aria-label', label('multi_scope_label', '表示範囲'));
  for (const [value, fallback] of [['box', 'この箱'], ['all', '全部']]) {
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.paneScope = value;
    button.textContent = label(value === 'box' ? 'multi_scope_box' : 'multi_scope_all', fallback);
    button.addEventListener('click', () => {
      manager()?.setScope?.(value);
      renderCompactPaneSwitcher();
    });
    scope.appendChild(button);
  }
  const selectLabel = document.createElement('label');
  selectLabel.className = 'compact-pane-label';
  selectLabel.textContent = label('pane_placement_visible_pane', '表示する画面');
  const select = document.createElement('select');
  select.id = 'compact-pane-select';
  select.setAttribute('aria-label', label('pane_placement_visible_pane', '表示する画面'));
  select.addEventListener('change', () => {
    const mgr = manager();
    const index = Number(select.value);
    if (mgr?.setMobileActiveSlot?.(index)) mgr.focusSlot?.(index);
  });
  selectLabel.appendChild(select);
  switcher.append(scope, selectLabel);
  stack.insertBefore(switcher, multiView);

  window.addEventListener('flexible-pane-layout-changed', renderCompactPaneSwitcher);
  window.matchMedia('(max-width: 1000px)').addEventListener('change', renderCompactPaneSwitcher);
  new MutationObserver(renderCompactPaneSwitcher).observe(multiView, { attributes: true, attributeFilter: ['hidden'] });
  renderCompactPaneSwitcher();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initPanePlacementMenu, { once: true });
} else {
  initPanePlacementMenu();
}

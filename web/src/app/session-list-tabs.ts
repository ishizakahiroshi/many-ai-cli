// The sidebar UI has one lazy owner; callbacks avoid importing state/settings and their cycles.
import { t } from '../i18n.js';
import type { SessionSnapshot } from '../types/proto.js';
import type { SidebarProjectNode } from './sidebar-tree.js';
import { SESSION_LIST_TAB_DRAG, SESSION_LIST_CARD_DRAG, SESSION_LIST_TABS_KEY,
  loadSessionListTabs, normalizeSessionListTabs, saveSessionListTabs, addSessionListTab,
  renameSessionListTab, deleteSessionListTab, reorderSessionListTab, reconcileSessionListTabs,
  sessionListIdentity, sessionListFamilies, sessionListOwners, canMoveSessionListFamily, moveSessionListFamily,
  filterSessionListTree, sessionListWaitingCounts, type SessionListTabState } from './session-list-tab-state.js';

interface Callbacks {
  sessions: () => SessionSnapshot[];
  render: () => void;
  cardMenu: (x: number, y: number, id: number, origin?: HTMLElement) => void;
  pending: (id: number) => boolean;
  resetDrag: () => void;
}
let callbacks: Callbacks | null = null;
let value: SessionListTabState | null = null;
let initialized = false;
let tabsSignature = '';
let restoreScroll = true;
let suppressScroll = false;
let suppressTapUntil = 0;
let heldPointerId: number | null = null;
let menu: HTMLElement | null = null;
let dialog: HTMLDialogElement | null = null;
let menuOrigin: HTMLElement | null = null;
let drag: { kind: 'tab'; id: string } | { kind: 'session'; id: number; identity: string } | null = null;
let edgeFrame = 0;
let edgeDirection = 0;
let touchTimer: ReturnType<typeof setTimeout> | null = null;
let touch: { x: number; y: number; pointerId: number; origin: HTMLElement; identity: string | null; fired: boolean } | null = null;
let storageFailed = false;
let preserveSelectionDepth = 0;

function tx(key: string, fallback: string, vars?: Record<string, unknown>): string {
  const translated = t(key, vars);
  return translated === key ? fallback.replace(/\{(\w+)\}/g, (_, name) => String(vars?.[name] ?? '')) : translated;
}
function state(): SessionListTabState {
  if (!value) {
    const name = tx('session_list_tab_name', 'Tab {number}', { number: 1 });
    try { value = loadSessionListTabs(localStorage, name); }
    catch { value = normalizeSessionListTabs(null, name); storageFailed = true; }
  }
  return value;
}
function all(): SessionSnapshot[] { return callbacks?.sessions() || []; }
function strip(): HTMLElement | null { return document.getElementById('session-list-tabs'); }
function scrollElement(): HTMLElement | null {
  return document.body.classList.contains('mobile-drawer-open')
    ? document.querySelector<HTMLElement>('.mobile-drawer-body') || document.getElementById('sessions')
    : document.getElementById('sessions');
}
function persist(): void {
  try { storageFailed = !saveSessionListTabs(localStorage, state()); }
  catch { storageFailed = true; }
  const notice = document.getElementById('session-list-tabs-storage');
  if (notice) {
    notice.hidden = !storageFailed;
    notice.textContent = tx('session_list_tabs_storage_error', 'Tab changes could not be saved. They remain available in this window.');
  }
}
function rememberScroll(): void {
  const element = scrollElement();
  if (element && !restoreScroll && !suppressScroll) state().scroll[state().selectedId] = element.scrollTop;
}
function revealSelected(): void {
  const parent = strip();
  const button = parent?.querySelector<HTMLElement>('[aria-selected="true"]');
  if (!parent || !button) return;
  const a = button.getBoundingClientRect(), b = parent.getBoundingClientRect();
  if (a.left < b.left) parent.scrollLeft -= b.left - a.left;
  else if (a.right > b.right) parent.scrollLeft += a.right - b.right;
}
function focusSelected(): void { strip()?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus(); }
export function sessionListCardElement(id: number): HTMLElement | null {
  return scrollElement()?.querySelector<HTMLElement>(`.card[data-session-id="${Number(id)}"], .mh-session-row[data-session-id="${Number(id)}"]`) || null;
}
export function restoreSessionListFocus(origin: HTMLElement | null): void {
  if (origin?.isConnected && (!origin.dataset.sessionId || scrollElement()?.contains(origin))) origin.focus();
  else if (origin?.dataset.sessionId) {
    const replacement = sessionListCardElement(Number(origin.dataset.sessionId));
    if (replacement) replacement.focus(); else focusSelected();
  } else if (origin?.dataset.sessionListTab) {
    const replacement = strip()?.querySelector<HTMLElement>(`[data-session-list-tab="${CSS.escape(origin.dataset.sessionListTab)}"]`);
    if (replacement) replacement.focus(); else focusSelected();
  } else focusSelected();
}
function refresh(): void {
  renderSessionListTabs();
  callbacks?.render();
  (window as any).renderMobileSessionDrawer?.(true);
  const element = scrollElement();
  if (element && restoreScroll) element.scrollTop = state().scroll[state().selectedId] || 0;
  restoreScroll = false;
  revealSelected();
}
function select(id: string, focus = false): void {
  if (!state().tabs.some(tab => tab.id === id)) return;
  rememberScroll();
  state().selectedId = id;
  restoreScroll = true;
  refresh();
  persist();
  if (focus) focusSelected();
}

/** Capture synchronously at the user's launch click, before risk/model dialogs or preference saves. */
export function captureSessionListTab(): string { return state().selectedId; }
/** Apply one server-confirmed lifecycle only; never replay over an existing/manual root membership. */
export function assignCorrelatedSessionListSpawn(id: number, startedAt: string, tabId: string, sessions: SessionSnapshot[]): boolean {
  const session = sessions.find(item => item.id === id && item.started_at === startedAt);
  if (!session || !canMoveSessionListFamily(sessions, id)) return false;
  const family = sessionListFamilies(sessions).get(id);
  if (!family) return false;
  // A child always inherits its real root, even if its own launch was requested elsewhere.
  if (family[0] !== id) return true;
  const key = sessionListIdentity(session);
  if (!key) return false;
  if (Object.prototype.hasOwnProperty.call(state().memberships, key)) return true;
  const target = state().tabs.some(tab => tab.id === tabId) ? tabId : state().defaultId;
  if (!moveSessionListFamily(state(), sessions, id, target)) return false;
  persist(); return true;
}

export function isSessionInSelectedListTab(id: number): boolean {
  return sessionListOwners(state(), all()).get(id) === state().selectedId;
}
export function sessionListTapSuppressed(): boolean { return Date.now() < suppressTapUntil || heldPointerId !== null; }
export function visibleSessionListTree(tree: SidebarProjectNode[], sessions: SessionSnapshot[]): SidebarProjectNode[] {
  return filterSessionListTree(tree, sessionListOwners(state(), sessions), state().selectedId);
}
/** Called by actual session activation only. Manual bucket selection never invokes activation. */
export function preserveSessionListSelection(action: () => void): void {
  preserveSelectionDepth++;
  try { action(); } finally { preserveSelectionDepth--; }
}
export function revealSessionListOwner(id: number): void {
  if (preserveSelectionDepth) return;
  const owner = sessionListOwners(state(), all()).get(id);
  if (owner && owner !== state().selectedId) select(owner);
}
/** Called after the mobile caller makes the drawer visible (hidden elements clamp scrollTop). */
export function restoreSessionListDrawerScroll(): void {
  const selected = state().selectedId;
  suppressScroll = true;
  const apply = () => {
    if (!document.body.classList.contains('mobile-drawer-open') || selected !== state().selectedId) return;
    const element = scrollElement();
    if (element) element.scrollTop = state().scroll[selected] || 0;
  };
  apply();
  requestAnimationFrame(() => { apply(); suppressScroll = false; });
}
/** Capture before render clears cards; do not save transient scroll clamping caused by DOM replacement. */
export function sessionListScrollBeforeRender(element: HTMLElement | null, openingMobile = false): number {
  suppressScroll = true;
  return restoreScroll || openingMobile && !document.body.classList.contains('mobile-drawer-open')
    ? state().scroll[state().selectedId] || 0 : element?.scrollTop || 0;
}
export function sessionListScrollAfterRender(element: HTMLElement | null, top: number, mobile = false): void {
  if (element) element.scrollTop = top;
  if (all().length && (mobile || element === scrollElement())) restoreScroll = false;
  requestAnimationFrame(() => { suppressScroll = false; });
}

export function initializeSessionListTabs(options: Callbacks): void {
  callbacks = options;
  if (initialized || !strip()) return;
  initialized = true;
  const parent = strip()!;
  const sidebar = document.getElementById('session-list')!;
  parent.setAttribute('aria-label', tx('session_list_tabs', 'Session list tabs'));
  document.getElementById('session-list-tab-add')?.addEventListener('click', () => {
    rememberScroll();
    const id = `tab-${Array.from(crypto.getRandomValues(new Uint32Array(4)), word => word.toString(16).padStart(8, '0')).join('')}`;
    addSessionListTab(state(), tx('session_list_tab_name', 'Tab {number}', { number: state().nextNumber }), id);
    restoreScroll = true; refresh(); persist(); focusSelected();
  });
  parent.addEventListener('click', event => {
    const button = (event.target as Element).closest<HTMLElement>('[data-session-list-tab]');
    if (button && !sessionListTapSuppressed()) select(button.dataset.sessionListTab!, true);
  });
  parent.addEventListener('keydown', event => {
    const button = (event.target as Element).closest<HTMLElement>('[data-session-list-tab]');
    if (!button || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const tabs = state().tabs, index = tabs.findIndex(tab => tab.id === button.dataset.sessionListTab);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1
      : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    select(tabs[next].id, true);
  });
  sidebar.addEventListener('contextmenu', event => {
    const origin = menuTarget(event.target);
    if (!origin) return;
    // Desktop cards keep their existing menu listener; tabs and mobile rows use the same menu here.
    if (!origin.dataset.sessionListTab && !origin.classList.contains('mh-session-row')) return;
    event.preventDefault(); event.stopPropagation(); openFor(origin, event.clientX, event.clientY);
  });
  sidebar.addEventListener('keydown', event => {
    const origin = menuTarget(event.target);
    if (!origin || !(event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey)) return;
    event.preventDefault(); event.stopPropagation();
    const rect = origin.getBoundingClientRect(); openFor(origin, rect.left, rect.bottom);
  });
  // Pointer movement/cancel leave native scrolling alone. Only a completed hold consumes its tap.
  sidebar.addEventListener('pointerdown', event => {
    if (event.pointerType !== 'touch' || event.button !== 0) return;
    cancelTouch();
    const origin = menuTarget(event.target);
    if (!origin || (event.target as Element).closest('.card-actions, .card-branch, .card-children-toggle')) return;
    touch = { x: event.clientX, y: event.clientY, pointerId: event.pointerId, origin, identity: origin.dataset.sessionId
      ? sessionListIdentity(all().find(session => session.id === Number(origin.dataset.sessionId))) : null, fired: false };
    touchTimer = setTimeout(() => {
      if (!touch) return;
      const original = touch.origin;
      if (original.dataset.sessionId && (!touch.identity ||
          sessionListIdentity(all().find(session => session.id === Number(original.dataset.sessionId))) !== touch.identity)) {
        // A reused numeric ID is a different gesture target. Consume release without opening its menu.
        heldPointerId = touch.pointerId; cancelTouch(); return;
      }
      // Live snapshots rebuild cards during a hold. Keep the identity, not the detached node.
      const current = original.isConnected ? original : original.dataset.sessionListTab
        ? sidebar.querySelector<HTMLElement>(`[data-session-list-tab="${CSS.escape(original.dataset.sessionListTab)}"]`)
        : sidebar.querySelector<HTMLElement>(`.card[data-session-id="${Number(original.dataset.sessionId)}"], .mh-session-row[data-session-id="${Number(original.dataset.sessionId)}"]`);
      if (!current) { cancelTouch(); return; }
      touch.fired = true; heldPointerId = touch.pointerId; suppressTapUntil = Date.now() + 1000;
      openFor(current, touch.x, touch.y);
    }, 550);
  }, true);
  sidebar.addEventListener('pointermove', event => {
    if (touch && (event.pointerId !== touch.pointerId || Math.hypot(event.clientX - touch.x, event.clientY - touch.y) > 10)) cancelTouch();
  }, { passive: true });
  sidebar.addEventListener('pointerup', event => {
    if (heldPointerId === event.pointerId) { suppressTapUntil = Date.now() + 450; event.preventDefault(); event.stopImmediatePropagation(); }
    heldPointerId = null; cancelTouch();
  }, true);
  sidebar.addEventListener('pointercancel', () => { heldPointerId = null; cancelTouch(); }, true);
  sidebar.addEventListener('click', event => {
    if (sessionListTapSuppressed() && menuTarget(event.target)) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
  document.addEventListener('scroll', event => {
    if (event.target === scrollElement()) { cancelTouch(); rememberScroll(); if (!suppressScroll) persist(); }
  }, true);
  window.addEventListener('pagehide', () => { rememberScroll(); persist(); clearDrag(); heldPointerId = null; cancelTouch(); });
  window.addEventListener('blur', () => { closeMenu(false); clearDrag(); heldPointerId = null; cancelTouch(); });
  document.addEventListener('pointerdown', event => { if (menu && !menu.contains(event.target as Node)) closeMenu(false); }, true);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') { clearDrag(); cancelTouch(); } });
  window.addEventListener('approval-queue-updated', renderSessionListTabs);
  document.addEventListener('i18n-ready', () => { tabsSignature = ''; renderSessionListTabs(); });
  bindDrag(parent, sidebar);
}
function menuTarget(target: EventTarget | null): HTMLElement | null {
  return target instanceof Element ? target.closest<HTMLElement>('[data-session-list-tab], .card[data-session-id], .mh-session-row[data-session-id]') : null;
}
function cancelTouch(): void {
  if (touchTimer) clearTimeout(touchTimer);
  touchTimer = null; touch = null;
}

export function renderSessionListTabs(): void {
  const parent = strip();
  if (!parent || !callbacks) return;
  if (reconcileSessionListTabs(state(), all())) persist();
  const counts = sessionListWaitingCounts(state(), all(), callbacks.pending);
  const signature = JSON.stringify([state().tabs, state().selectedId, [...counts]]);
  if (signature === tabsSignature || drag) return;
  tabsSignature = signature;
  const focused = parent.contains(document.activeElement);
  const x = parent.scrollLeft;
  parent.replaceChildren();
  for (const tab of state().tabs) {
    const button = document.createElement('button');
    button.type = 'button'; button.className = 'session-list-tab'; button.draggable = true;
    button.dataset.sessionListTab = tab.id; button.id = `session-list-tab-${tab.id}`;
    button.setAttribute('role', 'tab'); button.setAttribute('aria-controls', 'sessions');
    const selected = state().selectedId === tab.id;
    button.setAttribute('aria-selected', String(selected)); button.tabIndex = selected ? 0 : -1;
    button.title = tab.name;
    const name = document.createElement('span'); name.className = 'session-list-tab-name'; name.textContent = tab.name;
    button.append(name);
    const count = counts.get(tab.id) || 0;
    if (count && !selected) {
      const badge = document.createElement('span'); badge.className = 'session-list-tab-waiting'; badge.textContent = String(count);
      const label = tx('session_list_tab_waiting', '{count} waiting for a response', { count });
      badge.setAttribute('aria-label', label); button.title += ` · ${label}`; button.append(badge);
    }
    parent.append(button);
  }
  parent.scrollLeft = x;
  document.getElementById('sessions')?.setAttribute('aria-labelledby', `session-list-tab-${state().selectedId}`);
  const add = document.getElementById('session-list-tab-add');
  const label = tx('session_list_tab_add', 'Add tab');
  add?.setAttribute('title', label); add?.setAttribute('aria-label', label);
  if (focused) focusSelected();
  revealSelected();
}

function closeMenu(focus = true): void {
  menu?.remove(); menu = null;
  if (focus) restoreSessionListFocus(menuOrigin);
}
/** Reuse the card menu's visual language and provide keyboard alternatives to drag. */
function openFor(origin: HTMLElement, x: number, y: number): void {
  if (!origin.dataset.sessionListTab) { callbacks?.cardMenu(x, y, Number(origin.dataset.sessionId), origin); return; }
  closeMenu(false); menuOrigin = origin;
  menu = document.createElement('div'); menu.className = 'card-ctx-menu open session-list-tab-menu';
  menu.setAttribute('data-wheel-native', '');
  const id = origin.dataset.sessionListTab, index = state().tabs.findIndex(tab => tab.id === id);
  const item = (key: string, fallback: string, action: () => void, disabled = false) => {
    const button = document.createElement('button'); button.type = 'button'; button.textContent = tx(key, fallback); button.disabled = disabled;
    button.onclick = () => { closeMenu(false); action(); }; menu!.append(button);
  };
  item('session_list_tab_rename', 'Rename tab', () => renameDialog(id, origin));
  item('session_list_tab_left', 'Move left', () => { reorderSessionListTab(state(), id, state().tabs[index - 1].id, false); refresh(); persist(); focusSelected(); }, index <= 0);
  item('session_list_tab_right', 'Move right', () => { reorderSessionListTab(state(), id, state().tabs[index + 1].id, true); refresh(); persist(); focusSelected(); }, index === state().tabs.length - 1);
  item('session_list_tab_delete', 'Delete tab', () => deleteDialog(id, origin), state().tabs.length === 1);
  document.body.append(menu);
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, innerWidth - rect.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, innerHeight - rect.height - 4))}px`;
  wireSessionListMenu(menu, () => closeMenu());
}
export function wireSessionListMenu(element: HTMLElement, close: () => void): void {
  element.setAttribute('role', 'menu');
  element.querySelectorAll('button').forEach(button => button.setAttribute('role', 'menuitem'));
  element.addEventListener('keydown', event => {
    const controls = [...element.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key) || !controls.length) return;
    event.preventDefault(); event.stopPropagation();
    const index = controls.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? controls.length - 1
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + controls.length) % controls.length;
    controls[next].focus();
  });
  element.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
}
function openDialog(title: string, message: string, label: string, field: HTMLInputElement | HTMLSelectElement,
    action: () => boolean, origin: HTMLElement | null, submitText: string): void {
  if (dialog) return;
  dialog = document.createElement('dialog'); dialog.className = 'session-list-tab-dialog confirm-dialog aac-wheel-overlay';
  dialog.setAttribute('aria-labelledby', 'session-list-tab-dialog-title');
  const form = document.createElement('form');
  const heading = document.createElement('h2'); heading.className = 'confirm-title'; heading.id = 'session-list-tab-dialog-title'; heading.textContent = title;
  const description = document.createElement('p'); description.className = 'confirm-message'; description.textContent = message;
  const fieldLabel = document.createElement('label'); fieldLabel.textContent = label; fieldLabel.htmlFor = 'session-list-tab-dialog-value';
  field.id = fieldLabel.htmlFor; field.className = 'spawn-input';
  const error = document.createElement('p'); error.className = 'session-list-tab-error'; error.setAttribute('aria-live', 'polite');
  const actions = document.createElement('div'); actions.className = 'confirm-actions';
  const cancel = document.createElement('button'); cancel.type = 'button'; cancel.className = 'confirm-btn'; cancel.textContent = tx('confirm_cancel', 'Cancel');
  const submit = document.createElement('button'); submit.type = 'submit'; submit.className = 'confirm-btn primary'; submit.textContent = submitText;
  actions.append(cancel, submit); form.append(heading, description, fieldLabel, field, error, actions); dialog.append(form);
  const current = dialog;
  cancel.onclick = () => current.close();
  form.onsubmit = event => {
    event.preventDefault();
    if (!action()) { error.textContent = tx('session_list_tab_invalid', 'Enter a name or choose a valid destination.'); field.focus(); return; }
    refresh(); persist(); current.close();
  };
  current.addEventListener('keydown', event => event.stopPropagation());
  current.addEventListener('close', () => { current.remove(); dialog = null; restoreSessionListFocus(origin); });
  document.body.append(current); current.showModal(); field.focus();
  if (field instanceof HTMLInputElement) field.select();
}
function destinationSelect(exclude: string): HTMLSelectElement {
  const field = document.createElement('select');
  for (const tab of state().tabs) if (tab.id !== exclude) {
    const option = document.createElement('option'); option.value = tab.id; option.textContent = tab.name; field.append(option);
  }
  return field;
}
function renameDialog(id: string, origin: HTMLElement): void {
  const tab = state().tabs.find(tab => tab.id === id); if (!tab) return;
  const input = document.createElement('input'); input.value = tab.name;
  openDialog(tx('session_list_tab_rename', 'Rename tab'), '', tx('session_list_tab_label', 'Tab name'), input,
    () => renameSessionListTab(state(), id, input.value), origin, tx('confirm_ok', 'OK'));
}
function deleteDialog(id: string, origin: HTMLElement): void {
  if (state().tabs.length <= 1) return;
  const populated = [...sessionListOwners(state(), all()).values()].includes(id) || Object.values(state().memberships).includes(id);
  if (!populated) { rememberScroll(); deleteSessionListTab(state(), id, state().tabs.find(tab => tab.id !== id)!.id); restoreScroll = true; refresh(); persist(); focusSelected(); return; }
  const field = destinationSelect(id);
  openDialog(tx('session_list_tab_delete', 'Delete tab'), tx('session_list_tab_delete_help', 'Move all sessions to the selected tab, then delete this tab. Sessions, processes and history are kept.'),
    tx('session_list_tab_destination', 'Destination tab'), field, () => {
      rememberScroll(); const changed = deleteSessionListTab(state(), id, field.value); if (changed) restoreScroll = true; return changed;
    }, origin, tx('session_list_tab_delete_move', 'Move and delete'));
}
export function sessionListMoveLabel(id: number): string {
  if (!canMoveSessionListFamily(all(), id)) return tx('session_list_tab_incomplete', 'Wait for the parent session before moving this family');
  const family = sessionListFamilies(all()).get(id);
  return family && (family.length > 1 || family[0] !== id)
    ? tx('session_list_tab_move_family', 'Move root and children to another tab…')
    : tx('session_list_tab_move', 'Move to another tab…');
}
export function openSessionListMoveDialog(id: number, origin: HTMLElement | null): void {
  const selectedIdentity = sessionListIdentity(all().find(session => session.id === id));
  const rootId = sessionListFamilies(all()).get(id)?.[0];
  const rootIdentity = sessionListIdentity(all().find(session => session.id === rootId));
  const source = sessionListOwners(state(), all()).get(id);
  if (!source || state().tabs.length <= 1) return;
  const field = destinationSelect(source);
  openDialog(tx('session_list_tab_move', 'Move to another tab…'), tx('session_list_tab_family_help', 'The root session and all its children move together. Their project and parent relationships stay the same.'),
    tx('session_list_tab_destination', 'Destination tab'), field, () => {
      const currentRoot = sessionListFamilies(all()).get(id)?.[0];
      if (!selectedIdentity || sessionListIdentity(all().find(session => session.id === id)) !== selectedIdentity ||
          sessionListIdentity(all().find(session => session.id === currentRoot)) !== rootIdentity) return false;
      return moveSessionListFamily(state(), all(), id, field.value);
    }, origin, tx('confirm_ok', 'OK'));
}
export function sessionListCanMove(id: number): boolean {
  const root = sessionListFamilies(all()).get(id)?.[0];
  return state().tabs.length > 1 && canMoveSessionListFamily(all(), id) && !!sessionListIdentity(all().find(session => session.id === root));
}

function clearFeedback(): void { strip()?.querySelectorAll('.session-list-tab').forEach(tab => tab.classList.remove('drop-before', 'drop-after', 'drop-session')); }
function stopEdge(): void { cancelAnimationFrame(edgeFrame); edgeFrame = 0; edgeDirection = 0; }
function clearDrag(): void { drag = null; stopEdge(); clearFeedback(); renderSessionListTabs(); }
function edgeScroll(): void {
  if (!edgeDirection || !drag || !strip()) { stopEdge(); return; }
  strip()!.scrollLeft += edgeDirection * 5;
  edgeFrame = requestAnimationFrame(edgeScroll);
}
function bindDrag(parent: HTMLElement, sidebar: HTMLElement): void {
  sidebar.addEventListener('dragstart', event => {
    clearDrag(); cancelTouch(); closeMenu(false);
    const origin = (event.target as Element).closest<HTMLElement>('[data-session-list-tab], .card[data-session-id]');
    if (!origin || !event.dataTransfer) return;
    if (origin.dataset.sessionListTab) {
      callbacks?.resetDrag();
      drag = { kind: 'tab', id: origin.dataset.sessionListTab };
      event.dataTransfer.setData(SESSION_LIST_TAB_DRAG, JSON.stringify(drag));
      // Do not provide a pane payload for sidebar bucket reordering.
      event.dataTransfer.effectAllowed = 'move'; event.stopPropagation();
    } else {
      const id = Number(origin.dataset.sessionId), identity = sessionListIdentity(all().find(session => session.id === id));
      if (!identity) return;
      drag = { kind: 'session', id, identity };
      event.dataTransfer.setData(SESSION_LIST_CARD_DRAG, JSON.stringify(drag));
      // The existing card listener still supplies its distinct pane-placement payload.
    }
  }, true);
  parent.addEventListener('dragover', event => {
    if (!drag || !event.dataTransfer) return;
    const mime = drag.kind === 'tab' ? SESSION_LIST_TAB_DRAG : SESSION_LIST_CARD_DRAG;
    if (!event.dataTransfer.types.includes(mime)) return;
    event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect = 'move'; clearFeedback();
    const target = (event.target as Element).closest<HTMLElement>('[data-session-list-tab]');
    if (target) {
      const rect = target.getBoundingClientRect();
      target.classList.add(drag.kind === 'session' ? 'drop-session' : event.clientX > rect.left + rect.width / 2 ? 'drop-after' : 'drop-before');
    }
    const rect = parent.getBoundingClientRect();
    const direction = event.clientX < rect.left + 20 ? -1 : event.clientX > rect.right - 20 ? 1 : 0;
    if (direction !== edgeDirection) { stopEdge(); edgeDirection = direction; if (direction) edgeFrame = requestAnimationFrame(edgeScroll); }
  });
  parent.addEventListener('dragleave', event => { if (!parent.contains(event.relatedTarget as Node)) { stopEdge(); clearFeedback(); } });
  parent.addEventListener('drop', event => {
    if (!drag || !event.dataTransfer) return;
    const payload = drag, mime = payload.kind === 'tab' ? SESSION_LIST_TAB_DRAG : SESSION_LIST_CARD_DRAG;
    if (!event.dataTransfer.types.includes(mime)) { clearDrag(); return; }
    event.preventDefault(); event.stopPropagation();
    const target = (event.target as Element).closest<HTMLElement>('[data-session-list-tab]');
    if (target) {
      if (payload.kind === 'tab') {
        const rect = target.getBoundingClientRect(); reorderSessionListTab(state(), payload.id, target.dataset.sessionListTab!, event.clientX > rect.left + rect.width / 2);
      } else if (sessionListIdentity(all().find(session => session.id === payload.id)) === payload.identity) {
        moveSessionListFamily(state(), all(), payload.id, target.dataset.sessionListTab!);
      }
    }
    callbacks?.resetDrag();
    clearDrag(); refresh(); persist();
  });
  document.addEventListener('dragend', clearDrag, true);
  document.addEventListener('drop', clearDrag);
}

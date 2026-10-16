// Browser-local sidebar buckets. Never add these fields to user-prefs' full-snapshot PUT.
// Placement/families come from buildSidebarTree; tabs only filter that authoritative tree.
import type { SessionSnapshot } from '../types/proto.js';
import { buildSidebarTree, flattenSidebarTree, type SidebarProjectNode } from './sidebar-tree.js';

export const SESSION_LIST_TABS_KEY = 'many-ai-cli-session-list-tabs-v1';
export const SESSION_LIST_TAB_DRAG = 'application/x-many-ai-cli-session-list-tab';
export const SESSION_LIST_CARD_DRAG = 'application/x-many-ai-cli-session-list-card';
export interface SessionListTab { id: string; name: string; }
export interface SessionListTabState {
  version: 1;
  tabs: SessionListTab[];
  defaultId: string;
  selectedId: string;
  memberships: Record<string, string>;
  scroll: Record<string, number>;
  /** Per tab, the lifecycle identity of the session last opened while it belonged to that tab. */
  active: Record<string, string>;
}
type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;
const own = (value: object, key: string) => Object.prototype.hasOwnProperty.call(value, key);
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const validIdentity = (key: string) => /^session:[1-9]\d*:.{1,256}$/.test(key) && Number.isSafeInteger(Number(key.split(':')[1]));
const validTabId = (id: unknown): id is string => typeof id === 'string' && /^[a-zA-Z0-9_-]{1,100}$/.test(id);

/** Same lifecycle identity as pane placement. Missing timestamps never become durable memberships. */
export function sessionListIdentity(session: SessionSnapshot | undefined): string | null {
  if (!session || !Number.isSafeInteger(session.id) || session.id <= 0 ||
      typeof session.started_at !== 'string' || !session.started_at || session.started_at.length > 256) return null;
  return `session:${session.id}:${session.started_at}`;
}
export function normalizeSessionListTabs(raw: unknown, defaultName = 'Tab 1'): SessionListTabState {
  const source = record(raw);
  const data = source.version === undefined || source.version === 0 || source.version === 1 ? source : {};
  const tabs: SessionListTab[] = [];
  // Older local representations may keep memberships on each tab; bare numeric IDs are unsafe.
  const oldMembers: Array<[string, unknown[]]> = [];
  if (data.version === undefined || data.version === 0 || data.version === 1) {
    for (const candidate of Array.isArray(data.tabs) ? data.tabs : []) {
      const tab = record(candidate);
      if (!validTabId(tab.id) || tabs.some(item => item.id === tab.id)) continue;
      const name = typeof tab.name === 'string' ? tab.name.trim() : '';
      tabs.push({ id: tab.id, name: name || defaultName });
      if (Array.isArray(tab.sessions)) oldMembers.push([tab.id, tab.sessions]);
    }
  }
  if (!tabs.length) tabs.push({ id: 'tab-1', name: defaultName });
  const exists = (id: unknown): id is string => tabs.some(tab => tab.id === id);
  const defaultId = exists(data.defaultId) ? data.defaultId : tabs[0].id;
  const memberships: Record<string, string> = Object.create(null);
  for (const [id, keys] of oldMembers) {
    for (const key of keys) if (typeof key === 'string' && validIdentity(key) && !own(memberships, key)) memberships[key] = id;
  }
  for (const [key, id] of Object.entries(record(data.memberships))) {
    if (validIdentity(key)) memberships[key] = exists(id) ? id : defaultId;
  }
  const scroll: Record<string, number> = Object.create(null);
  for (const tab of tabs) {
    const value = record(data.scroll)[tab.id];
    scroll[tab.id] = typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0;
  }
  const active: Record<string, string> = Object.create(null);
  for (const tab of tabs) {
    const key = record(data.active)[tab.id];
    if (typeof key === 'string' && validIdentity(key)) active[tab.id] = key;
  }
  // An older nextNumber field (a lifetime counter) is dropped here and disappears on the next save.
  return { version: 1, tabs, defaultId, selectedId: exists(data.selectedId) ? data.selectedId : defaultId,
    memberships, scroll, active };
}
/** Numbered after the tabs that remain, so deleted tabs never push a fourth tab to "Tab 10"; a name still in use is skipped. */
export function nextSessionListTabName(state: SessionListTabState, format: (number: number) => string): string {
  const names = new Set(state.tabs.map(tab => tab.name));
  let number = state.tabs.length + 1;
  while (names.has(format(number))) number++;
  return format(number);
}
export function loadSessionListTabs(storage: StorageLike, defaultName: string): SessionListTabState {
  try { return normalizeSessionListTabs(JSON.parse(storage.getItem(SESSION_LIST_TABS_KEY) || 'null'), defaultName); }
  catch { return normalizeSessionListTabs(null, defaultName); }
}
export function saveSessionListTabs(storage: StorageLike, state: SessionListTabState): boolean {
  try { storage.setItem(SESSION_LIST_TABS_KEY, JSON.stringify(state)); return true; }
  catch { return false; }
}
export function addSessionListTab(state: SessionListTabState, name: string, uniqueId: string): SessionListTab | null {
  if (!name.trim() || !validTabId(uniqueId) || state.tabs.some(tab => tab.id === uniqueId)) return null;
  const tab = { id: uniqueId, name: name.trim() };
  state.tabs.push(tab);
  state.selectedId = tab.id;
  state.scroll[tab.id] = 0;
  return tab;
}
export function renameSessionListTab(state: SessionListTabState, id: string, name: string): boolean {
  const tab = state.tabs.find(tab => tab.id === id);
  if (!tab || !name.trim()) return false;
  tab.name = name.trim();
  return true;
}
/** Destination is mandatory even when currently disconnected: hidden saved members must survive too. */
export function deleteSessionListTab(state: SessionListTabState, id: string, destination: string): boolean {
  if (state.tabs.length <= 1 || id === destination || !state.tabs.some(tab => tab.id === id) ||
      !state.tabs.some(tab => tab.id === destination)) return false;
  for (const key of Object.keys(state.memberships)) {
    if (state.memberships[key] === id) state.memberships[key] = destination;
  }
  if (state.defaultId === id) state.defaultId = destination;
  if (state.selectedId === id) state.selectedId = destination;
  state.tabs = state.tabs.filter(tab => tab.id !== id);
  delete state.scroll[id];
  delete state.active[id];
  return true;
}
export function reorderSessionListTab(state: SessionListTabState, id: string, target: string, after: boolean): boolean {
  if (id === target || !state.tabs.some(tab => tab.id === id) || !state.tabs.some(tab => tab.id === target)) return false;
  const [tab] = state.tabs.splice(state.tabs.findIndex(tab => tab.id === id), 1);
  state.tabs.splice(state.tabs.findIndex(tab => tab.id === target) + (after ? 1 : 0), 0, tab);
  return true;
}
/** No pruning against an incomplete startup/reconnect map. Only observed ID reuse proves a stale identity. */
export function reconcileSessionListTabs(state: SessionListTabState, sessions: SessionSnapshot[]): boolean {
  let changed = false;
  const live = new Map(sessions.map(session => [session.id, sessionListIdentity(session)]));
  for (const key of Object.keys(state.memberships)) {
    const id = Number(key.split(':')[1]);
    const identity = live.get(id);
    if (identity && identity !== key) { delete state.memberships[key]; changed = true; }
  }
  return changed;
}
/** Whole families, including collapsed grandchildren, as decided by the one sidebar tree. */
export function sessionListFamilies(sessions: SessionSnapshot[]): Map<number, number[]> {
  const result = new Map<number, number[]>();
  for (const project of buildSidebarTree({ sessions, order: [] })) {
    for (const root of project.children) {
      const ids = flattenSidebarTree([{ ...project, children: [root] }]);
      for (const id of ids) result.set(id, ids);
    }
  }
  return result;
}
export function sessionListOwners(state: SessionListTabState, sessions: SessionSnapshot[]): Map<number, string> {
  const byId = new Map(sessions.map(session => [session.id, session]));
  const owners = new Map<number, string>();
  for (const [id, family] of sessionListFamilies(sessions)) {
    const key = sessionListIdentity(byId.get(family[0]));
    const saved = key ? state.memberships[key] : undefined;
    owners.set(id, state.tabs.some(tab => tab.id === saved) ? saved! : state.defaultId);
  }
  return owners;
}
/** A display orphan is visible, but it is not proof that the whole family can be moved. */
export function canMoveSessionListFamily(sessions: SessionSnapshot[], id: number): boolean {
  const byId = new Map(sessions.map(session => [session.id, session]));
  let current = byId.get(id);
  const seen = new Set<number>();
  while (current) {
    if (seen.has(current.id)) return false;
    seen.add(current.id);
    const parent = Number(current.parent_session_id || 0);
    if (!parent) return !!sessionListIdentity(current);
    current = byId.get(parent);
  }
  return false;
}
export function moveSessionListFamily(state: SessionListTabState, sessions: SessionSnapshot[], id: number, target: string): boolean {
  if (!state.tabs.some(tab => tab.id === target) || !canMoveSessionListFamily(sessions, id)) return false;
  const family = sessionListFamilies(sessions).get(id);
  if (!family) return false;
  const byId = new Map(sessions.map(session => [session.id, session]));
  const rootKey = sessionListIdentity(byId.get(family[0]));
  if (!rootKey) return false; // Wait for lifecycle metadata rather than keying a reusable ID.
  for (const member of family) {
    const key = sessionListIdentity(byId.get(member));
    if (key) state.memberships[key] = target;
  }
  return true;
}
/** Record an opened session under the tab that owns it. Without a lifecycle identity nothing is kept. */
export function rememberSessionListActive(state: SessionListTabState, sessions: SessionSnapshot[], id: number): boolean {
  const key = sessionListIdentity(sessions.find(session => session.id === id));
  const owner = sessionListOwners(state, sessions).get(id);
  if (!key || !owner || state.active[owner] === key) return false;
  state.active[owner] = key;
  return true;
}
/**
 * The session a tab shows when the user switches to it: the one last opened there while it still
 * belongs to the tab, otherwise the first in sidebar order. null only for a tab without sessions.
 */
export function sessionListTabSession(state: SessionListTabState, sessions: SessionSnapshot[], tabId: string, order: number[]): number | null {
  const owners = sessionListOwners(state, sessions);
  const key = state.active[tabId];
  const remembered = key ? sessions.find(session => sessionListIdentity(session) === key) : undefined;
  if (remembered && owners.get(remembered.id) === tabId) return remembered.id;
  return order.find(id => owners.get(id) === tabId) ?? null;
}
export function filterSessionListTree(tree: SidebarProjectNode[], owners: Map<number, string>, tabId: string): SidebarProjectNode[] {
  return tree.map(project => ({ ...project, children: project.children
    .filter(root => owners.get(root.id) === tabId)
    .map(root => ({ ...root, children: root.children.filter(child => owners.get(child.id) === tabId) })) }))
    .filter(project => project.children.length > 0);
}
function sessionListWaiting(session: SessionSnapshot, pending: (id: number) => boolean): boolean {
  return !!(session.awaiting_user || session.awaiting_approval || session.activity?.awaiting_user ||
    session.activity?.awaiting_approval || session.state === 'waiting' || pending(session.id));
}
export function sessionListWaitingCounts(state: SessionListTabState, sessions: SessionSnapshot[], pending: (id: number) => boolean = () => false): Map<string, number> {
  const counts = new Map(state.tabs.map(tab => [tab.id, 0]));
  const owners = sessionListOwners(state, sessions);
  for (const session of sessions) {
    if (sessionListWaiting(session, pending)) {
      const owner = owners.get(session.id) || state.defaultId;
      counts.set(owner, (counts.get(owner) || 0) + 1);
    }
  }
  return counts;
}

// Sessions that finished a turn and have not been opened since ("newly stopped"). The tab badge
// counts them and their cards draw the standby dot yellow, so both always agree. Browser-local like
// the tabs, keyed by lifecycle identity so a reused numeric ID never inherits a mark.
export const SESSION_LIST_STOPPED_KEY = 'many-ai-cli-session-list-stopped-v1';
const STOPPED_LIMIT = 200;
export function loadSessionListStopped(storage: StorageLike): Set<string> {
  try {
    const raw = JSON.parse(storage.getItem(SESSION_LIST_STOPPED_KEY) || 'null');
    const keys = (Array.isArray(raw) ? raw : []).filter((key): key is string => typeof key === 'string' && validIdentity(key));
    return new Set(keys.slice(-STOPPED_LIMIT));
  } catch { return new Set(); }
}
export function saveSessionListStopped(storage: StorageLike, unseen: Set<string>): boolean {
  try { storage.setItem(SESSION_LIST_STOPPED_KEY, JSON.stringify([...unseen])); return true; }
  catch { return false; }
}
/**
 * Record one observed turn end of a session the user is not looking at (the caller decides that).
 * Sessions in the selected tab are marked too: seeing the list is not opening the session.
 */
export function noteSessionListStopped(sessions: SessionSnapshot[], unseen: Set<string>, id: number): boolean {
  const key = sessionListIdentity(sessions.find(session => session.id === id));
  if (!key) return false;
  unseen.delete(key);
  unseen.add(key);
  while (unseen.size > STOPPED_LIMIT) unseen.delete(unseen.values().next().value!);
  return true;
}
/** The session was opened (shown in the terminal). Returns whether a mark was removed. */
export function clearSessionListStopped(sessions: SessionSnapshot[], unseen: Set<string>, id: number): boolean {
  const key = sessionListIdentity(sessions.find(session => session.id === id));
  return !!key && unseen.delete(key);
}
/** A marked session shows the mark only while it is plain standby (not running, not waiting on the user). */
export function isSessionListStoppedShown(session: SessionSnapshot | undefined, unseen: Set<string>,
    pending: (id: number) => boolean = () => false): boolean {
  const key = sessionListIdentity(session);
  return !!key && unseen.has(key) && (session!.state || 'standby') === 'standby' && !sessionListWaiting(session!, pending);
}
/**
 * One pass per render. A mark ends only when the session is opened (clearSessionListStopped) or its
 * ID is reused. Running again or waiting on the user hides it without ending it: a short redraw
 * must not erase it, and a session nobody opened is still unopened when it stops again.
 * Absent sessions keep their mark, like memberships, because startup/reconnect maps can be incomplete.
 */
export function settleSessionListStopped(state: SessionListTabState, sessions: SessionSnapshot[], unseen: Set<string>,
    pending: (id: number) => boolean = () => false): { counts: Map<string, number>; changed: boolean } {
  const counts = new Map(state.tabs.map(tab => [tab.id, 0]));
  const owners = sessionListOwners(state, sessions);
  const live = new Map(sessions.map(session => [session.id, session]));
  let changed = false;
  for (const key of [...unseen]) {
    const session = live.get(Number(key.split(':')[1]));
    if (!session) continue;
    if (sessionListIdentity(session) !== key) {
      unseen.delete(key);
      changed = true;
      continue;
    }
    if (!isSessionListStoppedShown(session, unseen, pending)) continue;
    const owner = owners.get(session.id) || state.defaultId;
    counts.set(owner, (counts.get(owner) || 0) + 1);
  }
  return { counts, changed };
}

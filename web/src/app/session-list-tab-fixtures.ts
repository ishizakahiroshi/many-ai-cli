import assert from 'node:assert/strict';
import test from 'node:test';
import type { SessionSnapshot } from '../types/proto.js';
import { buildSidebarTree, flattenSidebarTree } from './sidebar-tree.js';
import { SESSION_LIST_TABS_KEY, normalizeSessionListTabs as normalize, loadSessionListTabs, saveSessionListTabs,
  addSessionListTab as add, nextSessionListTabName as nextName, renameSessionListTab as rename, deleteSessionListTab as remove,
  reorderSessionListTab as reorder, sessionListIdentity as identity, reconcileSessionListTabs as reconcile,
  sessionListOwners as owners, moveSessionListFamily as move, filterSessionListTree as filter,
  sessionListProjectRoots as projectRoots, moveSessionListProject as moveProject,
  sessionListWaitingCounts as waiting, SESSION_LIST_STOPPED_KEY, loadSessionListStopped as loadStopped,
  saveSessionListStopped as saveStopped, noteSessionListStopped as noteStopped,
  clearSessionListStopped as clearStopped, isSessionListStoppedShown as shown,
  settleSessionListStopped as settle, rememberSessionListActive as remember,
  sessionListTabSession as pick } from './session-list-tab-state.js';
const session = (id: number, extra: Partial<SessionSnapshot> = {}): SessionSnapshot =>
  ({ id, started_at: `2026-10-08T01:00:${String(id).padStart(2, '0')}Z`, cwd: '/repos/example', ...extra } as SessionSnapshot);
const state = () => { const value = normalize(null); add(value, 'Work', 'work'); return value; };

test('first use: one localized tab, all existing sessions visible, no records persisted', () => {
  const s = normalize(null, 'タブ 1');
  assert.deepEqual(s.tabs, [{ id: 'tab-1', name: 'タブ 1' }]);
  assert.equal(owners(s, [session(1), session(2)]).get(2), s.defaultId);
  assert.deepEqual(Object.keys(s.memberships), []);
  assert.ok(!JSON.stringify(s).includes('cwd'));
});
test('malformed, future-version and missing storage recover safely', () => {
  for (const raw of [null, [], 4, 'x', { tabs: null }, { version: 42, tabs: [{ id: 'a', name: 'Future' }] }]) {
    const s = normalize(raw); assert.equal(s.tabs.length, 1); assert.equal(s.selectedId, s.defaultId);
  }
  assert.equal(loadSessionListTabs({ getItem: () => '{', setItem() {} }, 'Default').tabs[0].name, 'Default');
});
test('old stable memberships migrate, duplicates and bare IDs cannot hijack a lifecycle', () => {
  const key = identity(session(1))!;
  const s = normalize({ tabs: [{ id: 'a', name: ' A ', sessions: [key, 7, '7'] },
    { id: 'a', name: 'Duplicate' }, { id: 'b', name: 'B', sessions: [key] }], selectedId: 'missing',
    memberships: { [identity(session(2))!]: 'missing', bad: 'a' }, scroll: { a: -5, b: Infinity } });
  assert.equal(s.tabs.length, 2); assert.equal(s.tabs[0].name, 'A');
  assert.equal(s.memberships[key], 'a'); assert.equal(s.selectedId, 'a');
  assert.equal(s.memberships[identity(session(2))!], 'a');
  assert.equal(Object.keys(s.memberships).length, 2); assert.equal(s.scroll.b, 0);
});
test('add/rename reject blanks and duplicate IDs; names remain text data', () => {
  const s = state(); const before = JSON.stringify(s);
  assert.equal(add(s, ' ', 'new'), null); assert.equal(add(s, 'new', 'work'), null);
  assert.equal(rename(s, 'work', '\n '), false); assert.equal(JSON.stringify(s), before);
  assert.equal(rename(s, 'work', '<img src=x onerror=alert(1)>'), true);
  assert.equal(s.tabs[1].name, '<img src=x onerror=alert(1)>');
});
test('a new tab is numbered after the tabs that remain, skipping names in use; UUID IDs stay distinct after reload', () => {
  const format = (n: number) => `Tab ${n}`;
  let s = normalize(null, 'Tab 1');
  for (let i = 0; i < 8; i++) { add(s, nextName(s, format), `tab-x${i}`); remove(s, `tab-x${i}`, s.defaultId); }
  s = normalize({ ...JSON.parse(JSON.stringify(s)), nextNumber: 10 });
  assert.ok(!('nextNumber' in s));
  for (const name of ['🚀', '🏢']) add(s, name, `tab-${name.codePointAt(0)}`);
  assert.equal(nextName(s, format), 'Tab 4');
  rename(s, s.defaultId, 'Tab 4');
  assert.equal(nextName(s, format), 'Tab 5');
  const id = `tab-${crypto.randomUUID()}`;
  add(s, nextName(s, format), id); assert.notEqual(id, 'work'); assert.equal(s.selectedId, id);
});
test('last-tab deletion and invalid destination/cancel do not mutate state', () => {
  const s = normalize(null); const before = JSON.stringify(s);
  assert.equal(remove(s, s.defaultId, s.defaultId), false); assert.equal(JSON.stringify(s), before);
  const two = state(); const beforeTwo = JSON.stringify(two);
  assert.equal(remove(two, 'work', 'missing'), false); assert.equal(JSON.stringify(two), beforeTwo);
});
test('delete migrates absent as well as live memberships and the default', () => {
  const s = state(); const live = [session(1), session(2)];
  move(s, live, 1, 'work'); s.memberships[identity(session(99))!] = 'work';
  s.scroll.work = 100;
  assert.equal(remove(s, 'work', s.defaultId), true);
  assert.equal(s.memberships[identity(session(99))!], s.defaultId); assert.ok(!('work' in s.scroll));
  add(s, 'Destination', 'target');
  assert.equal(remove(s, s.defaultId, 'target'), true);
  assert.equal(owners(s, live).get(2), 'target');
});
test('reorder leaves default, memberships, selected tab and sibling ordering alone', () => {
  const s = state(); add(s, 'Other', 'other'); move(s, [session(1)], 1, 'work');
  const members = JSON.stringify(s.memberships);
  reorder(s, 'other', 'tab-1', false);
  assert.deepEqual(s.tabs.map(t => t.id), ['other', 'tab-1', 'work']);
  assert.equal(s.defaultId, 'tab-1'); assert.equal(s.selectedId, 'other');
  assert.equal(JSON.stringify(s.memberships), members);
  assert.equal(reorder(s, 'work', 'missing', false), false);
});
test('root/child/grandchild move atomically without changing session metadata', () => {
  const s = state(); const live = [session(1), session(2, { parent_session_id: 1, cwd: '/repos/child' }),
    session(3, { parent_session_id: 2 }), session(4)]; const before = JSON.stringify(live);
  assert.equal(move(s, live, 3, 'work'), true);
  assert.deepEqual([...owners(s, live).values()], ['work', 'work', 'work', 'tab-1']);
  assert.equal(JSON.stringify(live), before);
  const tree = buildSidebarTree({ sessions: live, order: [4, 1, 3, 2], projectFavorites: ['/repos/example'] });
  assert.deepEqual(flattenSidebarTree(filter(tree, owners(s, live), 'work')), [1, 3, 2]);
  assert.deepEqual(flattenSidebarTree(filter(tree, owners(s, live), 'tab-1')), [4]);
});
test('late child inherits root while independent roots in one repo can split', () => {
  const s = state(); move(s, [session(1), session(2)], 1, 'work');
  assert.equal(owners(s, [session(1), session(2), session(3, { parent_session_id: 1 })]).get(3), 'work');
  assert.equal(owners(s, [session(1), session(2)]).get(2), 'tab-1');
});
test('partial/empty initial snapshots and reconnect do not prune saved memberships', () => {
  const s = state(); move(s, [session(1), session(2)], 1, 'work'); const before = JSON.stringify(s);
  reconcile(s, []); reconcile(s, [session(2)]); reconcile(s, [session(1, { started_at: '' })]);
  assert.equal(JSON.stringify(s), before); assert.equal(owners(s, [session(1)]).get(1), 'work');
});
test('reused numeric IDs do not inherit membership and proven stale keys are removed', () => {
  const s = state(); move(s, [session(1)], 1, 'work');
  const replacement = session(1, { started_at: '2026-10-09T01:00:00Z' });
  assert.equal(owners(s, [replacement]).get(1), 'tab-1');
  assert.equal(reconcile(s, [replacement]), true); assert.equal(Object.keys(s.memberships).length, 0);
});
test('unknown lifecycle cannot be moved or durably assigned using a bare ID', () => {
  const s = state(); const partial = session(1, { started_at: '' });
  assert.equal(identity(partial), null); assert.equal(move(s, [partial], 1, 'work'), false);
  assert.equal(owners(s, [partial]).get(1), s.defaultId);
});
test('unassigned sessions remain visible regardless of selected tab; spawn correlation is intentionally not implemented', () => {
  const s = state(); assert.equal(s.selectedId, 'work');
  assert.equal(owners(s, [session(5)]).get(5), s.defaultId);
  remove(s, 'work', s.defaultId); assert.equal(owners(s, [session(5)]).get(5), s.defaultId);
});
test('waiting/approval counts derive from live state and count each session once', () => {
  const s = state(); const live = [session(1, { awaiting_user: true, awaiting_approval: true, state: 'waiting' }),
    session(2, { activity: { awaiting_approval: true, awaiting_user: false, output_idle: false, workflow_active: false } }), session(3), session(4, { state: 'running' })];
  move(s, live, 1, 'work'); move(s, live, 2, 'work');
  assert.equal(waiting(s, live, id => id === 3).get('work'), 2);
  assert.equal(waiting(s, live, id => id === 3).get('tab-1'), 1);
  assert.ok(!JSON.stringify(s).includes('awaiting'));
});
test('newly stopped: a mark waits for its session to be opened, not for its tab to be selected', () => {
  const s = state(); const live = [session(1, { state: 'standby' }), session(2, { state: 'standby' })];
  move(s, live, 2, 'work'); const unseen = new Set<string>();
  // Seeing the list is not opening the session, so the selected tab collects a mark too.
  assert.equal(noteStopped(live, unseen, 2), true);
  assert.equal(noteStopped(live, unseen, 1), true);
  const hidden = settle(s, live, unseen);
  assert.equal(hidden.counts.get('tab-1'), 1); assert.equal(hidden.counts.get('work'), 1); assert.equal(hidden.changed, false);
  s.selectedId = 'tab-1';
  const selected = settle(s, live, unseen);
  assert.equal(selected.changed, false); assert.equal(selected.counts.get('tab-1'), 1); assert.equal(unseen.size, 2);
  assert.equal(clearStopped(live, unseen, 1), true); assert.equal(clearStopped(live, unseen, 1), false);
  assert.equal(settle(s, live, unseen).counts.get('tab-1'), 0); assert.equal(unseen.size, 1);
  assert.equal(shown(live[0], unseen), false); assert.equal(shown(live[1], unseen), true);
});
test('newly stopped hides while running/waiting, ends on ID reuse, and survives an incomplete startup map', () => {
  const s = state(); const live = [session(1, { state: 'standby' })]; const unseen = new Set<string>();
  noteStopped(live, unseen, 1);
  // A short redraw makes a session "running" for ~3 s; it must not erase a mark nobody has opened.
  const running = settle(s, [session(1, { state: 'running' })], unseen);
  assert.equal(running.changed, false); assert.equal(running.counts.get('tab-1'), 0); assert.equal(unseen.size, 1);
  assert.equal(shown(session(1, { state: 'running' }), unseen), false);
  assert.equal(settle(s, live, unseen, id => id === 1).counts.get('tab-1'), 0); assert.equal(unseen.size, 1);
  assert.equal(shown(live[0], unseen, id => id === 1), false);
  assert.equal(settle(s, [session(1, { state: 'standby', awaiting_user: true })], unseen).counts.get('tab-1'), 0);
  assert.equal(settle(s, live, unseen).counts.get('tab-1'), 1); assert.equal(shown(live[0], unseen), true);
  assert.equal(settle(s, [session(1, { started_at: '2026-10-09T01:00:00Z', state: 'standby' })], unseen).counts.get('tab-1'), 0);
  assert.equal(unseen.size, 0);
  noteStopped(live, unseen, 1);
  assert.equal(settle(s, [], unseen).changed, false); assert.equal(unseen.size, 1);
  assert.equal(noteStopped([session(1, { started_at: '' })], new Set(), 1), false);
  assert.equal(clearStopped([session(1, { started_at: '' })], unseen, 1), false); assert.equal(unseen.size, 1);
});
test('newly stopped marks reload as identities only; storage denial stays usable', () => {
  let saved = ''; const storage = { getItem: (key: string) => { assert.equal(key, SESSION_LIST_STOPPED_KEY); return saved; },
    setItem: (key: string, value: string) => { assert.equal(key, SESSION_LIST_STOPPED_KEY); saved = value; } };
  const unseen = new Set([identity(session(1))!]);
  assert.equal(saveStopped(storage, unseen), true); assert.deepEqual([...loadStopped(storage)], [...unseen]);
  saved = JSON.stringify([7, 'bad', identity(session(2))]); assert.deepEqual([...loadStopped(storage)], [identity(session(2))]);
  saved = '{'; assert.equal(loadStopped(storage).size, 0);
  const denied = { getItem() { throw Error('denied'); }, setItem() { throw Error('quota'); } };
  assert.equal(loadStopped(denied).size, 0); assert.equal(saveStopped(denied, unseen), false);
});
test('reload preserves names/order/memberships/selected tab/scroll; storage denial stays usable', () => {
  const s = state(); s.scroll.work = 213; move(s, [session(1)], 1, 'work');
  let saved = ''; const storage = { getItem: (key: string) => { assert.equal(key, SESSION_LIST_TABS_KEY); return saved; },
    setItem: (key: string, value: string) => { assert.equal(key, SESSION_LIST_TABS_KEY); saved = value; } };
  assert.equal(saveSessionListTabs(storage, s), true);
  assert.deepEqual(loadSessionListTabs(storage, 'Tab 1'), s);
  const denied = { getItem() { throw Error('denied'); }, setItem() { throw Error('quota'); } };
  assert.equal(loadSessionListTabs(denied, 'Local').tabs[0].name, 'Local');
  assert.equal(saveSessionListTabs(denied, s), false); assert.equal(s.selectedId, 'work');
});

test('each tab remembers its last opened session; moves, ID reuse and empty tabs fall back safely', () => {
  const s = state(); const order = [1, 2, 3, 4];
  const live = [session(1), session(2), session(3), session(4, { parent_session_id: 3 })];
  move(s, live, 3, 'work');
  assert.equal(pick(s, live, 'work', order), 3);
  assert.equal(remember(s, live, 4), true); assert.equal(remember(s, live, 4), false);
  assert.equal(pick(s, live, 'work', order), 4);
  assert.equal(remember(s, live, 2), true); assert.equal(pick(s, live, 'tab-1', order), 2);
  move(s, live, 2, 'work'); assert.equal(pick(s, live, 'tab-1', order), 1);
  const reused = [session(1), session(2), session(3), session(4, { parent_session_id: 3, started_at: '2026-10-09T01:00:00Z' })];
  assert.equal(pick(s, reused, 'work', order), 2);
  assert.equal(remember(s, [session(5, { started_at: '' })], 5), false);
  add(s, 'Empty', 'empty'); assert.equal(pick(s, live, 'empty', order), null);
  assert.deepEqual(normalize(JSON.parse(JSON.stringify(s))).active, s.active);
  assert.deepEqual({ ...normalize({ tabs: [{ id: 'a', name: 'A' }], active: { a: 7, b: identity(session(1)) } }).active }, {});
  assert.equal(remove(s, 'work', s.defaultId), true); assert.ok(!('work' in s.active));
});
test('project header move takes only the selected tab roots of that project, families included', () => {
  const s = state(); add(s, 'Other', 'other'); add(s, 'Dest', 'dest'); s.selectedId = 'work';
  const live = [session(1), session(2, { parent_session_id: 1, cwd: '/repos/child' }), session(3),
    session(4, { cwd: '/repos/other' })];
  move(s, live, 1, 'work'); move(s, live, 3, 'other'); move(s, live, 4, 'work');
  assert.deepEqual(projectRoots(s, live, 'example', 'work'), [1]);
  assert.deepEqual(projectRoots(s, live, 'example', 'other'), [3]);
  assert.deepEqual(projectRoots(s, live, 'missing', 'work'), []);
  assert.equal(moveProject(s, live, 'example', 'work', 'dest'), true);
  assert.deepEqual([...owners(s, live).values()], ['dest', 'dest', 'other', 'work']);
  assert.deepEqual(projectRoots(s, live, 'example', 'work'), []);
  assert.deepEqual(projectRoots(s, live, 'example', 'dest'), [1]);
  assert.equal(moveProject(s, live, 'example', 'work', 'dest'), false);
  const before = JSON.stringify(s);
  assert.equal(moveProject(s, live, 'example', 'dest', 'missing'), false);
  assert.equal(JSON.stringify(s), before);
});
test('C2-R1: missing ancestor defers a child move and reconnect preserves the original family', () => {
  const s = state(); const root = session(1), child = session(2, { parent_session_id: 1 });
  move(s, [root, child], 1, 'work'); const before = JSON.stringify(s);
  assert.equal(move(s, [child], 2, 'tab-1'), false);
  assert.equal(JSON.stringify(s), before);
  assert.equal(owners(s, [child]).get(2), 'work');
  assert.equal(owners(s, [root, child]).get(2), 'work');
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { SessionStartupStore, terminalHasVisibleContent } from '../src/app/session-startup.js';
import { addPaneContent, normalizeFlexibleLayouts, paneContentKey } from '../src/app/flexible-pane-state.js';
const source = (name: string) => readFileSync(new URL('../src/app/' + name, import.meta.url), 'utf8');
const compile = (text: string) => new Bun.Transpiler({ loader: 'ts' }).transformSync(text.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, ''));
const tick = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
const identity = { id: 7, startedAt: 'synthetic-start' };
test('registration alone stays starting, rendered content from the exact lifecycle completes it', () => {
  const store = new SessionStartupStore(); store.begin('request-A', 'claude', 'Claude Code', 'tab-A'); store.bind('request-A', identity);
  assert.equal(store.requests.get('request-A')?.phase, 'starting');
  store.outputPainted({ id: 7, startedAt: 'previous-start' }); store.outputPainted({ id: 8, startedAt: 'synthetic-start' });
  assert.equal(store.requests.get('request-A')?.phase, 'starting');
  store.outputPainted(identity); assert.equal(store.requests.get('request-A')?.phase, 'output');
});
test('output before correlation is remembered; failure/lost HTTP cannot override proven registration', () => {
  const store = new SessionStartupStore(); store.begin('request-A', 'claude', 'Claude Code', 'tab-A'); store.outputPainted(identity);
  store.outcome('request-A', false); store.bind('request-A', identity); store.outcome('request-A', true, 'rejected');
  assert.equal(store.requests.get('request-A')?.phase, 'output');
});
test('slow launch and disconnection remain pending; reconnect and Hub replacement have distinct lifetimes', () => {
  const store = new SessionStartupStore(); store.snapshot('hub-A', []);
  store.begin('request-A', 'claude', 'Claude Code', 'tab-A'); store.markSlow('request-A'); store.disconnected();
  assert.equal(store.requests.get('request-A')?.phase, 'confirming');
  store.snapshot('hub-A', []); assert.ok(store.requests.has('request-A'));
  store.snapshot('hub-B', []); assert.equal(store.requests.size, 0);
});
test('explicit rejection is local to its request and late registration still wins', () => {
  const store = new SessionStartupStore(); store.begin('A', 'claude', 'Claude Code', 'tab-A'); store.begin('B', 'codex', 'Codex', 'tab-B');
  store.outcome('A', true, 'synthetic rejection'); assert.equal(store.requests.get('A')?.phase, 'failed');
  assert.equal(store.requests.get('B')?.phase, 'starting'); store.bind('A', identity); assert.equal(store.requests.get('A')?.phase, 'starting');
  store.dismiss('B'); assert.equal(store.requests.has('B'), false);
});
test('blank/ANSI-only screen never ends feedback while visible terminal cells do', () => {
  let line = ''; const term = { rows: 2, buffer: { active: { viewportY: 3, getLine: () => ({ translateToString: () => line }) } } };
  assert.equal(terminalHasVisibleContent(term), false); line = '   '; assert.equal(terminalHasVisibleContent(term), false);
  line = 'Claude Code'; assert.equal(terminalHasVisibleContent(term), true);
});
test('startup pane grows without replacing existing content and never restores as a session', () => {
  const first = { kind: 'session' as const, sessionId: 1, startedAt: 'first' };
  const pending = { kind: 'startup' as const, requestId: 'A', sessionId: null, startedAt: null };
  const layout = { cols: 1, rows: 1, colFracs: [1], rowFracs: [1], slots: [first] as any[] };
  const index = addPaneContent(layout, pending); assert.equal(index, 1); assert.equal(layout.slots[0], first);
  assert.equal(paneContentKey(pending), 'startup:A');
  const restored = normalizeFlexibleLayouts({ all: layout }); assert.equal(restored.all.slots[1], null);
});
test('actual reservation handoff stays in its saved layout and does not focus or switch a later view', () => {
  const text = source('multi-pane.ts'); const start = text.indexOf('  reconcileStartups('), end = text.indexOf('\n  /** Move', start);
  const context: any = {}; runInNewContext(compile('class Manager {' + text.slice(start, end) + '}\nglobalThis.Manager=Manager;'), context);
  const startup = { kind: 'startup', requestId: 'A', sessionId: null, startedAt: null };
  const other = { kind: 'session', sessionId: 2, startedAt: 'other' };
  const manager = new context.Manager(); manager.customLayouts = { all: { slots: [other, startup] }, 'box:other': { slots: [other] } };
  manager.area = { hidden: true }; manager._layoutKey = () => 'box:other'; let saves = 0; manager._saveCustomLayouts = () => saves++; manager.render = () => { throw Error('hidden view must not render'); };
  const store = new SessionStartupStore(); store.begin('A', 'claude', 'Claude Code', 'tab-A'); store.bind('A', identity);
  manager.reconcileStartups(store.requests); assert.equal(manager.customLayouts.all.slots[0], other);
  assert.equal(manager.customLayouts.all.slots[1].sessionId, 7); assert.equal(manager.customLayouts['box:other'].slots[0], other); assert.equal(saves, 1);
});
test('dismissed startup UI does not consume a later registration and state remains bounded', () => {
  const store = new SessionStartupStore(); store.begin('A', 'claude', 'Claude Code', 'tab-A'); store.dismiss('A'); store.bind('A', identity);
  assert.equal(store.requests.has('A'), false);
  for (let i = 0; i < 300; i++) { store.begin('synthetic-' + i, 'claude', 'Claude Code', 'tab-A'); store.outcome('synthetic-' + i, true); }
  assert.equal(store.requests.size, 128);
});

test('actual startup reservation keeps the same focused session when grid growth changes indices', () => {
  const text = source('multi-pane.ts'), start = text.indexOf('  addStartup('), end = text.indexOf('\n  reconcileStartups(', start);
  const context: any = {}; runInNewContext(compile('class Manager {' + text.slice(start, end) + '}\nglobalThis.Manager=Manager;'), context);
  const sessions = Array.from({ length: 6 }, (_, i) => ({ id: i + 1, started_at: 'synthetic-' + (i + 1) }));
  const manager = new context.Manager(); manager.area = { hidden: false }; manager.focusedIdx = 3;
  manager.slots = sessions.map(session => ({ session })); manager._contentForPayload = (value: any) => value;
  let focused = -1; manager.focusSlot = (idx: number) => { focused = idx; };
  manager._addContent = () => {
    const layout = { cols: 2, rows: 3, colFracs: [1, 1], rowFracs: [1, 1, 1], slots: sessions.map(session => ({ kind: 'session', sessionId: session.id, startedAt: session.started_at })) as any[] };
    addPaneContent(layout, { kind: 'startup', requestId: 'synthetic-new', sessionId: null, startedAt: null });
    manager.slots = layout.slots.map(slot => slot?.kind === 'session' ? { session: sessions.find(item => item.id === slot.sessionId) } : null);
    return true;
  };
  assert.equal(manager.addStartup('synthetic-new'), true);
  assert.equal(manager.slots[focused]?.session.id, 4, 'input must remain on the same session after a new column is inserted');
});
function spawnFixture(options: { status?: number; pendingHttp?: boolean; rejectNetwork?: boolean } = {}) {
  // The filename below names the public UI module exercised by this fixture.
  const moduleSource = source('spawn-panel.ts'); // doxguard: allow panel
  const start = moduleSource.indexOf('  async function spawnSession()');
  const end = moduleSource.indexOf('\n  }', start) + 4;
  const no = () => {}; const store = new SessionStartupStore(); let resolveHttp: (value: any) => void = no;
  const calls: string[] = []; const inputs: any = { 'spawn-provider': { value: 'claude' }, 'spawn-label': { value: 'Synthetic' }, 'spawn-permission-mode': { value: 'default' } };
  const context: any = { spawnLaunchSequence: 0, captureSpawnTab: () => 'tab-A', document: { getElementById: (id: string) => inputs[id] || { value: '' } },
    spawnCwdInput: { value: '/repos/example' }, spawnLaunchBlockReason: () => '', isPathMissing: () => false, commandMissingProviderIds: new Set(), spawnLaunchBtn: { disabled: false }, spawnModelInput: { value: '' },
    getSpawnOpenTarget: () => 'main', getSpawnGridLayout: () => '1x1', spawnDetachedPreset: { value: 'single' }, updateSpawnLaunchButton: no,
    getCachedSpawnModelGroups: () => [], isModelCompatibleWithProvider: () => true, resolveRoute: () => '', isCustomProviderValue: () => false,
    readSpawnDefaults: () => ({}), hasProviderBooleanSetting: () => false, spawnInitialPrompt: { value: '' }, selectedSubscriptionID: () => '', effortForSpawnBody: () => '', selectedEffort: () => '',
    claudeModelSelection: null, codexModelSelection: null, isClaudeHighRisk: () => false, spawnOrchestrationMode: false,
    beginTrackedSpawn: () => 'A', beginSessionStartup: (id: string, provider: string, name: string, tab: string) => { calls.push('pane'); store.begin(id, provider, name, tab); return true; }, noteSessionStartupOutcome: (id: string, definite: boolean, detail: string) => store.outcome(id, definite, detail),
    apiFetch: async () => { calls.push('http'); if (options.rejectNetwork) throw Error('synthetic disconnect'); if (options.pendingHttp) return new Promise(resolve => { resolveHttp = resolve; }); return { ok: (options.status || 200) === 200, status: options.status || 200 }; },
    saveCwdHistory: no, spawnSubscriptionRow: { hidden: true }, spawnIsolateWorktree: { checked: false }, spawnDelegation: { checked: false }, spawnRememberApprovalSettings: { checked: true },
    mergeProviderBooleanSetting: (value: any) => value, mergeApprovalSettings: (value: any) => value,
    saveSpawnSettings: () => { calls.push('preferences'); return new Promise(() => {}); }, newSessionPanel: { hidden: false }, setSpawnOrchestrationMode: no,
    waitForTrackedSpawn: () => { calls.push('legacy-wait'); return Promise.resolve([7]); }, activateSession: () => { calls.push('activate'); }, spawnErrorMessage: async () => 'synthetic rejection',
    alert: () => { calls.push('alert'); }, showToast: no, t: (key: string) => key, window: {} };
  runInNewContext(compile(moduleSource.slice(start, end)) + '\nglobalThis.spawn=spawnSession;', context);
  return { context, calls, store, resolveHttp: (value: any) => resolveHttp(value) };
}
test('actual click paints pending UI before slow HTTP, and slow prefs do not hold completion', async () => {
  const fixture = spawnFixture({ pendingHttp: true }); const pending = fixture.context.spawn(); await tick();
  assert.deepEqual(fixture.calls, ['pane', 'http']); assert.equal(fixture.context.newSessionPanel.hidden, true);
  fixture.resolveHttp({ ok: true, status: 200 }); await pending;
  assert.ok(fixture.calls.includes('preferences')); assert.ok(!fixture.calls.includes('legacy-wait')); assert.ok(!fixture.calls.includes('activate'));
});
test('actual click distinguishes definitive refusal from transport uncertainty without retry', async () => {
  const refused = spawnFixture({ status: 403 }); await refused.context.spawn(); assert.equal(refused.store.requests.get('A')?.phase, 'failed');
  const lost = spawnFixture({ rejectNetwork: true }); await lost.context.spawn(); assert.equal(lost.store.requests.get('A')?.phase, 'confirming');
  assert.equal(lost.calls.filter(value => value === 'http').length, 1); assert.ok(!lost.calls.includes('alert'));
});

test('startup provider label belongs to the submitted launch when confirmation outlives a selection', async () => {
  const fixture = spawnFixture();
  const provider = fixture.context.document.getElementById('spawn-provider');
  provider.selectedOptions = [{ textContent: 'Claude Code' }];
  fixture.context.isClaudeHighRisk = () => true;
  fixture.context.appConfirm = async () => { provider.value = 'codex'; provider.selectedOptions = [{ textContent: 'Codex' }]; return true; };
  await fixture.context.spawn();
  assert.equal(fixture.store.requests.get('A')?.provider, 'claude');
  assert.equal(fixture.store.requests.get('A')?.name, 'Claude Code');
});
test('cancelling existing risk confirmation never begins startup UI or sends HTTP', async () => {
  const fixture = spawnFixture(); fixture.context.isClaudeHighRisk = () => true; fixture.context.appConfirm = async () => false;
  await fixture.context.spawn(); assert.deepEqual(fixture.calls, []); assert.equal(fixture.store.requests.size, 0);
});
test('actual view cleans failed reservations and does not take over a newer sidebar tab', () => {
  const store = new SessionStartupStore(); let currentTab = 'tab-A', clears = 0, placements = 0, switches = 0; const notices: string[] = [];
  const context: any = { sessionStartups: store, captureSessionListTab: () => currentTab, setTimeout: () => 1, clearTimeout: () => clears++,
    window: { multiPaneManager: { addStartup: () => { placements++; return false; }, reconcileStartups: () => {} } },
    document: { querySelectorAll: () => [], getElementById: () => ({ hidden: true }) }, setActiveTab: () => switches++, activeSessionId: 1, showToast: (text: string) => notices.push(text), t: (key: string) => key };
  runInNewContext(compile(source('session-startup-view.ts')) + '\nglobalThis.begin=beginSessionStartup;globalThis.outcome=noteSessionStartupOutcome;globalThis.timers=slowTimers;', context);
  assert.equal(context.begin('A', 'claude', 'Claude Code', 'tab-A'), false); assert.equal(store.requests.size, 0); assert.equal(context.timers.size, 0); assert.equal(clears, 1);
  currentTab = 'tab-B'; assert.equal(context.begin('B', 'claude', 'Claude Code', 'tab-A'), true);
  assert.equal(placements, 1); assert.equal(switches, 0); assert.equal(store.requests.get('B')?.tabId, 'tab-A');
  context.outcome('B', true, 'synthetic refusal'); assert.equal(notices.length, 1); assert.match(notices[0], /synthetic refusal/);
  store.dismiss('B'); assert.equal(context.timers.size, 0); context.outcome('B', true, 'closed pane refusal'); assert.equal(notices.length, 2);
});
test('old terminal flush cannot complete a reused session ID or replacement terminal', () => {
  const text = source('terminal.ts'), start = text.indexOf('export function writePTYChunk'), end = text.indexOf('// ---- バッファスキャン', start);
  let flush: () => void = () => {}; const terminal = { rows: 1, buffer: { active: { viewportY: 0, getLine: () => ({ translateToString: () => 'old output' }) } }, write: (_bytes: unknown, callback: () => void) => { flush = callback; } };
  const oldEntry: any = {}; const entries = new Map([[7, oldEntry]]), sessions = new Map([[7, { started_at: 'old-start' }]]);
  const store = new SessionStartupStore(); store.begin('A', 'claude', 'Claude Code', 'tab-A'); store.bind('A', identity);
  const pass = (_id: number, bytes: unknown) => bytes, no = () => {};
  const context: any = { terminals: entries, sessions, sessionStartups: store, terminalHasVisibleContent, scanMouseModePure: () => ({}), initialMouseModeTrackerState: () => ({}),
    detectScreenClearSeqForAutoScroll: () => false, filterBareCarriageReturnForDisplay: pass, filterCursorHideShowBlocksForDisplay: pass, filterEraseScrollbackForDisplay: pass,
    filterReverseVideoForDisplay: pass, filterHubMarkersForDisplay: pass, primeScrollbarVisibility: no, hasPendingAltScrollNotch: () => false, updateAltScrollRail: no };
  runInNewContext(compile(text.slice(start, end)) + '\nglobalThis.write=writePTYChunk;', context);
  context.write(7, terminal, new Uint8Array([65]), null);
  entries.set(7, {}); sessions.set(7, { started_at: identity.startedAt }); flush();
  assert.equal(store.requests.get('A')?.phase, 'starting'); assert.equal(oldEntry.startupOutputPainted, undefined);
});
test('actual grid attachment creates the terminal when correlation arrives before PTY', () => {
  const text = source('multi-pane.ts'), start = text.indexOf('  attachToSlot('), end = text.indexOf('\n  detachSlot(', start);
  const events: string[] = [], frames: (() => void)[] = []; let entry: any = null;
  const children: any[] = []; const area = { isConnected: true, appendChild: (child: any) => { children.push(child); }, contains: (child: any) => children.includes(child) };
  const context: any = { mountTerminalStartup: () => events.push('feedback'),
    ensureTerminal: () => { events.push('ensure'); entry = { term: { open: () => events.push('open') }, autoScroll: false }; },
    document: { createElement: () => ({ style: {}, clientWidth: 400, clientHeight: 250 }) }, requestAnimationFrame: (frame: () => void) => frames.push(frame),
    ensureAltScrollRail: () => {}, ensureAltDragSelect: () => {}, enableWebglRenderer: () => {},
    window: { getTerminalEntry: () => { events.push('lookup'); return entry; }, flushPendingTerminalChunks: () => events.push('flush') } };
  runInNewContext(compile('class Manager {' + text.slice(start, end) + '}\nglobalThis.Manager=Manager;'), context);
  const manager = new context.Manager(); manager._shouldForceBottomOnAttach = () => false; manager._ensureScrollHandler = () => {};
  manager._installResizeObserver = () => {}; manager._fitTerminalInSlot = () => {};
  manager.attachToSlot({ querySelector: () => area }, { id: 7, started_at: identity.startedAt });
  assert.deepEqual(events, ['feedback', 'ensure', 'lookup']);
  assert.equal(frames.length, 1); frames[0](); assert.equal(entry.everAttached, true); assert.deepEqual(events.slice(-2), ['open', 'flush']);
});
test('older accepted HTTP never clears or hides a reopened form or captures its new toggles', async () => {
  const fixture = spawnFixture({ pendingHttp: true }); let persisted: any; fixture.context.saveSpawnSettings = (value: any) => { persisted = value; return Promise.resolve(); };
  fixture.context.mergeProviderBooleanSetting = (base: any, _provider: string, values: any) => ({ ...base, ...values });
  const pending = fixture.context.spawn(); await tick();
  fixture.context.newSessionPanel.hidden = false; fixture.context.spawnInitialPrompt.value = 'new unsent prompt'; fixture.context.spawnIsolateWorktree.checked = true;
  fixture.resolveHttp({ ok: true, status: 200 }); await pending;
  assert.equal(fixture.context.newSessionPanel.hidden, false); assert.equal(fixture.context.spawnInitialPrompt.value, 'new unsent prompt');
  assert.equal(persisted.isolate_worktree, false);
});

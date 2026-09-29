import assert from 'node:assert/strict';
import test from 'node:test';
import {
  addPaneContent,
  migrateSessionTabsToWorkspaces,
  normalizeFlexibleLayout,
  normalizeFlexibleLayouts,
  nextPaneDimensions,
  paneContentKey,
  paneLayoutKey,
  parsePaneDragPayload,
  previewPaneDrop,
  sessionPaneLayoutKey,
} from './flexible-pane-state.js';

const session = (id: number, startedAt = 'synthetic-start') =>
  ({ kind: 'session', sessionId: id, startedAt });
const tab = (tabName: string, id: number | null = null) =>
  ({ kind: 'tab', tabName, sessionId: id, startedAt: id === null ? null : 'synthetic-start' });
const layout = (slots: unknown[], cols = 2, rows = 1) =>
  ({ cols, rows, colFracs: Array(cols).fill(1), rowFracs: Array(rows).fill(1), slots });

test('scope keys keep all and each open box independent', () => {
  assert.equal(paneLayoutKey('all', '/work/alpha'), 'all');
  assert.equal(paneLayoutKey('box', '/work/alpha'), 'box:/work/alpha');
  assert.equal(paneLayoutKey('box', '/work/bravo'), 'box:/work/bravo');
  assert.equal(paneLayoutKey('box', null), 'all');
});

test('saved layout rejects duplicate session and singleton chat placements', () => {
  const duplicateSession = normalizeFlexibleLayout(layout([session(1), session(1)]));
  assert.deepEqual(duplicateSession?.slots, [session(1), null]);
  const duplicateChat = normalizeFlexibleLayout(layout([tab('chat', 1), tab('chat', 2)]));
  assert.deepEqual(duplicateChat?.slots, [tab('chat', 1), null]);
  assert.notEqual(paneContentKey(tab('files', 1) as any), paneContentKey(tab('files', 2) as any));
});

test('unknown and non-placeable tabs become empty slots', () => {
  const normalized = normalizeFlexibleLayout(layout([tab('unknown'), tab('multi')]));
  assert.deepEqual(normalized?.slots, [null, null]);
  // 廃止した split タブを保存していた古い配置も、空きスロットへ落ちる
  assert.deepEqual(normalizeFlexibleLayout(layout([tab('split'), tab('approval')]))?.slots,
    [null, tab('approval')]);
  assert.deepEqual(normalizeFlexibleLayout(layout([tab('terminal', 1), null]))?.slots,
    [null, null]);
});

test('dimensions are bounded and invalid fractions reset without losing slots', () => {
  assert.equal(normalizeFlexibleLayout(layout([], 7, 1)), null);
  assert.equal(normalizeFlexibleLayout(layout([], 1, 0)), null);
  const invalidFractions = { ...layout([session(1), null]), colFracs: [0, NaN] };
  assert.deepEqual(normalizeFlexibleLayout(invalidFractions)?.colFracs, [1, 1]);
  assert.deepEqual(normalizeFlexibleLayout(layout([session(1)], 2, 1))?.slots,
    [session(1), null]);
  assert.deepEqual(Object.keys(normalizeFlexibleLayouts({ all: layout([session(1), null]),
    'box:/work/alpha': layout([null, tab('approval')]), bogus: layout([]) })).sort(),
    ['all', 'box:/work/alpha']);
});

test('drag payload accepts only known tab and live-session shapes', () => {
  assert.deepEqual(parsePaneDragPayload('{"kind":"session","sessionId":3}'),
    { kind: 'session', sessionId: 3 });
  assert.deepEqual(parsePaneDragPayload('{"kind":"tab","tabName":"files","sessionId":3}'),
    { kind: 'tab', tabName: 'files', sessionId: 3 });
  for (const raw of ['garbage', '{}', '{"kind":"session","sessionId":-1}',
    '{"kind":"tab","tabName":"multi"}', '{"kind":"tab","tabName":"unknown"}']) {
    assert.equal(parsePaneDragPayload(raw), null);
  }
});

test('automatic pane growth adds capacity through the supported grid sizes', () => {
  assert.deepEqual(nextPaneDimensions(2, 1), { cols: 2, rows: 2 });
  assert.deepEqual(nextPaneDimensions(2, 2), { cols: 3, rows: 2 });
  assert.deepEqual(nextPaneDimensions(3, 2), { cols: 3, rows: 3 });
  assert.deepEqual(nextPaneDimensions(6, 3), null);
});

test('drop adds to an empty slot and expands a full grid without replacing a pane', () => {
  const current = layout([session(1), session(2), session(3), session(4)], 2, 2);
  assert.equal(addPaneContent(current as any, session(5) as any), 2);
  assert.deepEqual([current.cols, current.rows], [3, 2]);
  assert.deepEqual(current.slots, [session(1), session(2), session(5), session(3), session(4), null]);
  assert.equal(addPaneContent(current as any, session(3) as any), 3);
  assert.equal(current.slots.filter(Boolean).length, 5);
  assert.equal(addPaneContent(current as any, session(6) as any, 5), 5);
  assert.deepEqual(normalizeFlexibleLayout(current)?.slots, current.slots);
});

test('session workspaces restore only their own terminal and related views', () => {
  const key1 = sessionPaneLayoutKey(1, 'start-a');
  const key2 = sessionPaneLayoutKey(2, 'start-b');
  const restored = normalizeFlexibleLayouts({
    [key1]: layout([session(1, 'start-a'), { ...tab('git', 1), startedAt: 'start-a' }, tab('files', 2)], 3, 1),
    [key2]: layout([session(2, 'start-b'), tab('files', 1)], 2, 1),
  });
  assert.deepEqual(restored[key1]?.slots, [session(1, 'start-a'), { ...tab('git', 1), startedAt: 'start-a' }, null]);
  assert.deepEqual(restored[key2]?.slots, [session(2, 'start-b'), null]);
});

test('preview predicts growth and the chosen new cell is the actual destination', () => {
  const before = layout([session(1), tab('files', 1)], 2, 1);
  const prediction = previewPaneDrop(before as any, tab('git', 1) as any);
  assert.deepEqual([prediction?.layout.cols, prediction?.layout.rows], [2, 2]);
  assert.deepEqual(prediction?.layout.slots, [session(1), tab('files', 1), null, null]);
  assert.deepEqual(before.slots, [session(1), tab('files', 1)]);
  assert.equal(addPaneContent(before as any, tab('git', 1) as any, 3), 3);
  assert.deepEqual(before.slots, [session(1), tab('files', 1), null, tab('git', 1)]);
});

test('mixed legacy layouts move session tabs without changing overview terminal order', () => {
  const layouts = normalizeFlexibleLayouts({
    all: layout([session(1), tab('files', 1), session(2), tab('git', 2)], 2, 2),
  });
  assert.equal(migrateSessionTabsToWorkspaces(layouts), true);
  assert.deepEqual(layouts.all.slots, [session(1), null, session(2), null]);
  assert.deepEqual(layouts[sessionPaneLayoutKey(1, 'synthetic-start')].slots,
    [session(1), tab('files', 1)]);
  assert.deepEqual(layouts[sessionPaneLayoutKey(2, 'synthetic-start')].slots,
    [session(2), tab('git', 2)]);
  assert.equal(migrateSessionTabsToWorkspaces(layouts), false);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  normalizeFlexibleLayout,
  normalizeFlexibleLayouts,
  paneContentKey,
  paneLayoutKey,
  parsePaneDragPayload,
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

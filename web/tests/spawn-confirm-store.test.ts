import { beforeEach, describe, expect, test } from 'bun:test';
import {
  clearAllSpawnConfirmationsForHubRestart,
  closeSpawnConfirmation,
  isDialogControllerOpen,
  noteSpawnConfirmationRequested,
  pendingSpawnConfirmationCount,
  registerDialogController,
  selectOldestPendingConfirmation,
  selectOldestPendingConfirmationFor,
  spawnConfirmDecisionFromHttp,
  SPAWN_CONFIRM_CLOSED_FALLBACK_MS,
  unregisterDialogController,
  type SpawnConfirmationRecord,
} from '../src/app/spawn-confirm-store.ts';

// plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C3.
// This module is deliberately DOM-free so it can be imported directly from
// bun:test without pulling in session-list.ts's much larger import graph
// (see the file header comment in spawn-confirm-store.ts for why).

beforeEach(() => {
  // The store is a module-level singleton; reset it between tests so one
  // test's confirmations never leak into the next.
  clearAllSpawnConfirmationsForHubRestart();
});

function requestedMessage(overrides: Record<string, unknown> = {}) {
  return {
    spawn_confirmation_id: 'sc-1',
    session_id: 10,
    role: 'implementation',
    provider: 'claude',
    model: '',
    cwd: '',
    initial_prompt: 'do the thing',
    spawn_requested_at_ms: 1000,
    ...overrides,
  };
}

describe('noteSpawnConfirmationRequested / pendingSpawnConfirmationCount', () => {
  test('adds a record and counts it only for its own parent', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1 }));
    expect(pendingSpawnConfirmationCount(1)).toBe(1);
    expect(pendingSpawnConfirmationCount(2)).toBe(0);
  });

  test('a resend with the same confirmation id overwrites rather than duplicates', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1, spawn_requested_at_ms: 1000 }));
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1, spawn_requested_at_ms: 2000 }));
    expect(pendingSpawnConfirmationCount(1)).toBe(1);
  });

  test('ignores a message with no confirmation id', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: '', session_id: 1 }));
    expect(pendingSpawnConfirmationCount(1)).toBe(0);
  });

  test('counts multiple confirmations for the same parent (different roles)', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1, role: 'implementation' }));
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-b', session_id: 1, role: 'review' }));
    expect(pendingSpawnConfirmationCount(1)).toBe(2);
  });
});

describe('closeSpawnConfirmation', () => {
  test('removes the record from the store', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1 }));
    expect(pendingSpawnConfirmationCount(1)).toBe(1);
    closeSpawnConfirmation({ spawn_confirmation_id: 'sc-a', session_id: 1, reason: 'refused' });
    expect(pendingSpawnConfirmationCount(1)).toBe(0);
  });

  test('is a no-op for an unknown id', () => {
    expect(() => closeSpawnConfirmation({ spawn_confirmation_id: 'sc-does-not-exist', reason: 'refused' })).not.toThrow();
  });

  test('delegates to a registered dialog controller and forwards the message', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1 }));
    let received: any = null;
    registerDialogController('sc-a', { applyClosed: (m) => { received = m; } });
    closeSpawnConfirmation({ spawn_confirmation_id: 'sc-a', session_id: 1, reason: 'approved', spawn_child_session_id: 42 });
    expect(received).toEqual({ spawn_confirmation_id: 'sc-a', session_id: 1, reason: 'approved', spawn_child_session_id: 42 });
    unregisterDialogController('sc-a');
  });

  test('does nothing to a dialog controller once it has been unregistered', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1 }));
    let calls = 0;
    registerDialogController('sc-a', { applyClosed: () => { calls++; } });
    unregisterDialogController('sc-a');
    closeSpawnConfirmation({ spawn_confirmation_id: 'sc-a', session_id: 1, reason: 'refused' });
    expect(calls).toBe(0);
  });
});

describe('selectOldestPendingConfirmation', () => {
  function record(id: string, parentId: number, requestedAtMs: number): SpawnConfirmationRecord {
    return { id, parentId, role: '', provider: '', model: '', cwd: '', initialPrompt: '', requestedAtMs };
  }

  test('picks the oldest (smallest requestedAtMs) confirmation for the given parent', () => {
    const records = new Map<string, SpawnConfirmationRecord>([
      ['a', record('a', 1, 3000)],
      ['b', record('b', 1, 1000)],
      ['c', record('c', 1, 2000)],
      ['d', record('d', 2, 500)], // different parent — must never win
    ]);
    const picked = selectOldestPendingConfirmation(records, 1, () => false);
    expect(picked?.id).toBe('b');
  });

  test('skips confirmations whose dialog is already open', () => {
    const records = new Map<string, SpawnConfirmationRecord>([
      ['a', record('a', 1, 1000)],
      ['b', record('b', 1, 2000)],
    ]);
    const picked = selectOldestPendingConfirmation(records, 1, (id) => id === 'a');
    expect(picked?.id).toBe('b');
  });

  test('returns null when nothing is pending for the parent', () => {
    const records = new Map<string, SpawnConfirmationRecord>([['a', record('a', 1, 1000)]]);
    expect(selectOldestPendingConfirmation(records, 2, () => false)).toBeNull();
  });

  test('selectOldestPendingConfirmationFor reads from the module-level store and respects open dialogs', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-old', session_id: 1, spawn_requested_at_ms: 1000 }));
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-new', session_id: 1, spawn_requested_at_ms: 2000 }));
    expect(selectOldestPendingConfirmationFor(1)?.id).toBe('sc-old');
    registerDialogController('sc-old', { applyClosed: () => {} });
    expect(isDialogControllerOpen('sc-old')).toBe(true);
    expect(selectOldestPendingConfirmationFor(1)?.id).toBe('sc-new');
    unregisterDialogController('sc-old');
  });
});

describe('clearAllSpawnConfirmationsForHubRestart', () => {
  test('empties the store and closes any open dialog controllers with parent_gone', () => {
    noteSpawnConfirmationRequested(requestedMessage({ spawn_confirmation_id: 'sc-a', session_id: 1 }));
    let reason = '';
    registerDialogController('sc-a', { applyClosed: (m) => { reason = String(m?.reason || ''); } });
    clearAllSpawnConfirmationsForHubRestart();
    expect(pendingSpawnConfirmationCount(1)).toBe(0);
    expect(reason).toBe('parent_gone');
    unregisterDialogController('sc-a');
  });
});

describe('spawn confirm HTTP 200 without WS close', () => {
  test('POST 200 waits for spawn_confirmation_closed and has a fallback timeout', () => {
    const decision = spawnConfirmDecisionFromHttp(true, 200);
    expect(decision.waitForCloseBroadcast).toBe(true);
    expect(decision.fallbackMs).toBe(SPAWN_CONFIRM_CLOSED_FALLBACK_MS);
    expect(decision.terminalReason).toBe('');
    expect(SPAWN_CONFIRM_CLOSED_FALLBACK_MS).toBeGreaterThan(0);
  });

  test('HTTP errors still become terminal with Close and do not wait for WS', () => {
    expect(spawnConfirmDecisionFromHttp(false, 404)).toEqual({
      waitForCloseBroadcast: false,
      fallbackMs: 0,
      terminalReason: 'expired',
    });
    expect(spawnConfirmDecisionFromHttp(false, 409).terminalReason).toBe('decided_elsewhere');
    expect(spawnConfirmDecisionFromHttp(false, 500).terminalReason).toBe('submit_failed');
  });
});


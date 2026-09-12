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
  approvalForSelection,
  recordFromMessage,
  setLaunchOptionChoices,
  effortLevelsFor,
  effortForSpawnBody,
  headlessUnsupportedForSelection,
  isExecutionModeAvailable,
  isHeadlessCapable,
  isPermissionPresetAvailable,
  rememberedRolePermission,
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

// The dialog is the one place a human sees what the child is being granted, so
// the approval preview has to survive the trip through the store. Missing or
// malformed input must degrade to "nothing added" rather than throwing — an
// older Hub does not send the field at all.
describe('spawn_child_approval', () => {
  test('is parsed per provider and per permission tier', () => {
    const rec = recordFromMessage(
      requestedMessage({
        spawn_child_approval: {
          codex: {
            '': { sandbox: 'danger-full-access', ask_for_approval: 'never', risk_confirmed: true, tier: 'full' },
            bounded: { sandbox: 'workspace-write', ask_for_approval: 'never', risk_confirmed: true, tier: 'bounded' },
          },
          claude: {
            '': { permission_mode: 'bypassPermissions', risk_confirmed: true, tier: 'full' },
            bounded: { permission_mode: 'dontAsk', allowed_tools: ['Read', 'Bash(git log *)'], risk_confirmed: true, tier: 'bounded' },
          },
          grok: {
            bounded: { permission_mode: 'bypassPermissions', risk_confirmed: true, tier: 'full', fallback_from: 'bounded' },
          },
        },
      }),
    );
    expect(rec.approval.codex['']).toEqual({
      permissionMode: '',
      sandbox: 'danger-full-access',
      askForApproval: 'never',
      allowedTools: [],
      riskConfirmed: true,
      tier: 'full',
      fallbackFrom: '',
    });
    expect(rec.approval.claude.bounded.permissionMode).toBe('dontAsk');
    expect(rec.approval.claude.bounded.allowedTools).toEqual(['Read', 'Bash(git log *)']);
    // 段 2 が無い provider は段 3 へ落ちる。落ちたことが残っていないと、
    // ダイアログは「範囲を限った」と読ませたまま全許可の子を起こす。
    expect(rec.approval.grok.bounded.tier).toBe('full');
    expect(rec.approval.grok.bounded.fallbackFrom).toBe('bounded');
  });

  test('is empty when the Hub sends nothing', () => {
    expect(recordFromMessage(requestedMessage()).approval).toEqual({});
  });

  test('ignores a malformed payload instead of throwing', () => {
    expect(recordFromMessage(requestedMessage({ spawn_child_approval: 'nope' })).approval).toEqual({});
    expect(recordFromMessage(requestedMessage({ spawn_child_approval: { codex: null } })).approval.codex).toEqual({});
    const partial = recordFromMessage(requestedMessage({ spawn_child_approval: { codex: { bounded: null } } }));
    expect(partial.approval.codex.bounded).toEqual({
      permissionMode: '',
      sandbox: '',
      askForApproval: '',
      allowedTools: [],
      riskConfirmed: false,
      tier: '',
      fallbackFrom: '',
    });
  });

  // ダイアログは provider と段のどちらを差し替えても表示を引き直す。組み合わせが
  // 無ければ「要求どおり」へ落として、権限欄を空白のままにしない。
  test('picks the entry for the selected provider and tier', () => {
    const approval = recordFromMessage(
      requestedMessage({
        spawn_child_approval: {
          claude: {
            '': { permission_mode: 'bypassPermissions', tier: 'full' },
            bounded: { permission_mode: 'dontAsk', tier: 'bounded' },
          },
        },
      }),
    ).approval;
    expect(approvalForSelection(approval, 'claude', 'bounded')?.permissionMode).toBe('dontAsk');
    expect(approvalForSelection(approval, 'claude', '')?.permissionMode).toBe('bypassPermissions');
    expect(approvalForSelection(approval, 'claude', 'attended')?.permissionMode).toBe('bypassPermissions');
    expect(approvalForSelection(approval, 'codex', 'bounded')).toBeUndefined();
  });
});

// 起動要求の共通 3 項目（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C4 / C5）。
describe('launch options', () => {
  test('carries the requested values into the record', () => {
    const rec = recordFromMessage(
      requestedMessage({ effort: 'high', execution_mode: 'interactive', permission_preset: 'attended' }),
    );
    expect(rec.effort).toBe('high');
    expect(rec.executionMode).toBe('interactive');
    expect(rec.permissionPreset).toBe('attended');
  });

  test('leaves them empty when the Hub sends nothing', () => {
    const rec = recordFromMessage(requestedMessage());
    expect(rec.effort).toBe('');
    expect(rec.executionMode).toBe('');
    expect(rec.permissionPreset).toBe('');
  });

  test('takes the choices from /api/info and keeps unmapped providers empty', () => {
    setLaunchOptionChoices({
      effort_levels: { claude: ['low', 'high'], codex: ['minimal'] },
      execution_modes: ['auto', 'interactive'],
      permission_presets: ['attended', 'full'],
    });
    expect(effortLevelsFor('claude')).toEqual(['low', 'high']);
    expect(effortLevelsFor('copilot')).toEqual([]);
    expect(isExecutionModeAvailable('interactive')).toBe(true);
    expect(isExecutionModeAvailable('headless')).toBe(false);
    expect(isPermissionPresetAvailable('full')).toBe(true);
    expect(isPermissionPresetAvailable('bounded')).toBe(false);
  });

  test('ignores a malformed /api/info payload instead of throwing', () => {
    setLaunchOptionChoices({ effort_levels: 'nope', execution_modes: 'nope' });
    expect(effortLevelsFor('claude')).toEqual(['low', 'high']);
    expect(isExecutionModeAvailable('interactive')).toBe(true);
  });
});

// headless の定義がある provider（子 plan:
// docs/local/plan_child_execution_modes_headless.md 内部 C6）。押す前に「この CLI には
// headless の定義が無い」を出すためだけの一覧で、受理の判断は常に Hub 側。
describe('headless capability', () => {
  test('names the providers /api/info listed, and only those', () => {
    setLaunchOptionChoices({ headless_providers: ['claude', 'grok'] });
    expect(isHeadlessCapable('claude')).toBe(true);
    expect(isHeadlessCapable('grok')).toBe(true);
    expect(isHeadlessCapable('codex')).toBe(false);
    expect(isHeadlessCapable('  claude  ')).toBe(true);
  });

  test('says yes when the list is unknown, so it never claims a capable CLI cannot', () => {
    setLaunchOptionChoices({ headless_providers: 'nope' });
    setLaunchOptionChoices({ headless_providers: [] });
    expect(isHeadlessCapable('anything')).toBe(true);
  });

  test('only an explicit headless is refused; auto falls back on the Hub side', () => {
    setLaunchOptionChoices({ headless_providers: ['claude'] });
    expect(headlessUnsupportedForSelection('codex', 'headless')).toBe(true);
    expect(headlessUnsupportedForSelection('codex', 'auto')).toBe(false);
    expect(headlessUnsupportedForSelection('codex', '')).toBe(false);
    expect(headlessUnsupportedForSelection('claude', 'headless')).toBe(false);
  });
});

// New Session フォームが送る effort（子 plan 内部 C5）。写像が無い provider では
// キーごと送らない＝従来の起動と 1 バイトも変わらない。
describe('effortForSpawnBody', () => {
  test('sends the selected level for a mapped provider', () => {
    setLaunchOptionChoices({ effort_levels: { claude: ['low', 'high'] } });
    expect(effortForSpawnBody('claude', 'high')).toBe('high');
  });

  test('sends nothing for a provider without a mapping', () => {
    setLaunchOptionChoices({ effort_levels: { claude: ['low', 'high'] } });
    expect(effortForSpawnBody('copilot', 'high')).toBe('');
  });

  test('sends nothing when nothing is selected or the level is unknown here', () => {
    setLaunchOptionChoices({ effort_levels: { claude: ['low', 'high'], codex: ['minimal'] } });
    expect(effortForSpawnBody('claude', '')).toBe('');
    expect(effortForSpawnBody('claude', '   ')).toBe('');
    // 前の provider で選んだ値が残っていても、候補に無ければ送らない。
    expect(effortForSpawnBody('codex', 'high')).toBe('');
  });
});

// 「この役割では次回もこの段を使う」（子 plan:
// docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6）。
describe('remember_permission', () => {
  test('opens the checkbox ON when the Hub already remembers this role', () => {
    expect(recordFromMessage(requestedMessage({ remember_permission: true })).rememberPermission).toBe(true);
  });

  test('is off when the Hub says so', () => {
    expect(recordFromMessage(requestedMessage({ remember_permission: false })).rememberPermission).toBe(false);
  });

  // 欄を持たない Hub からのメッセージでも落ちず、「覚えていない」と同じ見え方になる。
  test('is off when the field is absent', () => {
    expect(recordFromMessage(requestedMessage()).rememberPermission).toBe(false);
  });
});

// 派生ダイアログが役割を選んだ時点の段（/api/info の role_permission）。
describe('rememberedRolePermission', () => {
  test('returns the remembered tier for that role', () => {
    setLaunchOptionChoices({
      permission_presets: ['attended', 'bounded', 'full'],
      role_permission: { review: 'bounded', implementation: 'full' },
    });
    expect(rememberedRolePermission('review')).toBe('bounded');
    expect(rememberedRolePermission('implementation')).toBe('full');
  });

  test('returns nothing for a role with no memory', () => {
    setLaunchOptionChoices({
      permission_presets: ['attended', 'bounded', 'full'],
      role_permission: { review: 'bounded' },
    });
    expect(rememberedRolePermission('test')).toBe('');
    expect(rememberedRolePermission('')).toBe('');
  });

  // 選択肢に無い段を初期値にすると、見えている段と送る段が食い違う。
  test('drops a tier this build cannot select', () => {
    setLaunchOptionChoices({
      permission_presets: ['attended', 'full'],
      role_permission: { review: 'bounded' },
    });
    expect(rememberedRolePermission('review')).toBe('');
  });

  test('ignores a malformed payload instead of throwing', () => {
    setLaunchOptionChoices({
      permission_presets: ['attended', 'bounded', 'full'],
      role_permission: { review: 'bounded' },
    });
    setLaunchOptionChoices({ role_permission: 'nope' });
    expect(rememberedRolePermission('review')).toBe('bounded');
    setLaunchOptionChoices({ role_permission: { review: null, '': 'full' } });
    expect(rememberedRolePermission('review')).toBe('');
  });
});

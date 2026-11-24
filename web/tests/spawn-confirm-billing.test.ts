// Issue #14: the payment choice in the spawn confirmation dialog.
// Synthetic values only; no key or key command appears anywhere.
import { beforeEach, describe, expect, test } from 'bun:test';
import {
  billingDecision,
  billingFieldShown,
  billingFromMessageData,
  billingProfilesFor,
  billingRefusalDetail,
  clearAllSpawnConfirmationsForHubRestart,
  noteSpawnConfirmationRequested,
  recordFromMessage,
  refreshSpawnConfirmationDisclosure,
} from '../src/app/spawn-confirm-store.js';

const REVISION = 'a'.repeat(64);
function encode(value: unknown): string {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}
const disclosure = (extra: Record<string, unknown> = {}) => encode({
  version: 1,
  billing_mode: 'plan-login',
  billing_profile: '',
  max_budget_usd: null,
  billing_profiles: [{ id: 'synthetic', display_name: 'Synthetic', provider: 'claude', max_budget_usd: 5 }],
  billing_revision: REVISION,
  billing_unavailable: null,
  ...extra,
});

describe('billing disclosure', () => {
  test('an older Hub without data means the plan login and no choice', () => {
    for (const data of [undefined, '', 'not base64 !', encode({ version: 2 }), encode('x')]) {
      const billing = billingFromMessageData(data);
      expect(billing.mode).toBe('plan-login');
      expect(billing.profiles).toEqual([]);
      expect(billing.revision).toBe('');
      expect(billingFieldShown(billing, 'claude')).toBe(false);
    }
  });

  test('profiles are shown only for their CLI, and an AI request for API is always shown', () => {
    const billing = billingFromMessageData(disclosure());
    expect(billing.revision).toBe(REVISION);
    expect(billingProfilesFor(billing, 'claude').map((p) => p.id)).toEqual(['synthetic']);
    expect(billingFieldShown(billing, 'claude')).toBe(true);
    expect(billingFieldShown(billing, 'codex')).toBe(false);
    const asked = billingFromMessageData(disclosure({
      billing_mode: 'api-key', billing_profile: 'synthetic', max_budget_usd: 0.5, billing_profiles: [],
    }));
    expect(asked.mode).toBe('api-key');
    expect(asked.maxBudgetUsd).toBe(0.5);
    expect(billingFieldShown(asked, 'codex')).toBe(true);
  });

  test('malformed fields are dropped instead of shown', () => {
    const billing = billingFromMessageData(disclosure({
      billing_mode: 'something-else',
      billing_revision: 'not-a-revision',
      max_budget_usd: -1,
      billing_profiles: [
        { id: 'ok', display_name: 'OK', provider: 'claude', max_budget_usd: 5 }, { id: 1 }, null,
        { id: 'x'.repeat(200), display_name: 'X', provider: 'claude', max_budget_usd: 5 },
        // A profile without its own limit (or with 0, a string) cannot be offered.
        { id: 'nolimit', display_name: 'N', provider: 'claude' },
        { id: 'zero', display_name: 'Z', provider: 'claude', max_budget_usd: 0 },
        { id: 'text', display_name: 'T', provider: 'claude', max_budget_usd: '5' },
      ],
    }));
    expect(billing.mode).toBe('plan-login');
    expect(billing.revision).toBe('');
    expect(billing.maxBudgetUsd).toBeUndefined();
    expect(billing.profiles.map((p) => p.id)).toEqual(['ok']);
  });
});

describe('billing decision', () => {
  const base = {
    shown: true, approved: true, mode: 'api-key', executionMode: 'headless',
    profile: 'synthetic', budgetText: '0.5', revision: REVISION,
  };
  test('API key billing carries the shown revision and the budget, for this run only', () => {
    expect(billingDecision(base)).toEqual({
      fields: { billing_mode: 'api-key', billing_profile: 'synthetic', max_budget_usd: 0.5, billing_revision: REVISION },
    });
  });
  test('missing headless, profile or a positive budget stops before sending', () => {
    for (const change of [
      { executionMode: 'interactive' }, { executionMode: '' }, { profile: '' },
      { budgetText: '' }, { budgetText: '0' }, { budgetText: '-1' }, { budgetText: 'abc' },
    ]) {
      expect(billingDecision({ ...base, ...change })).toEqual({ error: 'headless_billing_invalid' });
    }
  });
  test('a budget above the limit the person wrote for that setting stops before sending', () => {
    expect(billingProfilesFor(billingFromMessageData(disclosure()), 'claude')[0].maxBudgetUsd).toBe(5);
    expect(billingDecision({ ...base, budgetText: '5', profileMaxUsd: 5 })).toEqual({
      fields: { billing_mode: 'api-key', billing_profile: 'synthetic', max_budget_usd: 5, billing_revision: REVISION },
    });
    for (const budgetText of ['5.01', '1e12']) {
      expect(billingDecision({ ...base, budgetText, profileMaxUsd: 5 })).toEqual({ error: 'headless_billing_invalid' });
    }
  });
  test('the plan login is sent explicitly when the field is shown, nothing when hidden or refused', () => {
    expect(billingDecision({ ...base, mode: 'plan-login' })).toEqual({ fields: { billing_mode: 'plan-login' } });
    expect(billingDecision({ ...base, shown: false })).toBeUndefined();
    expect(billingDecision({ ...base, approved: false })).toBeUndefined();
  });
});

describe('refreshed disclosure after a changed profile file', () => {
  beforeEach(() => clearAllSpawnConfirmationsForHubRestart());
  const message = (id: string, parent: number, data: string) => ({
    type: 'spawn_confirmation_requested', spawn_confirmation_id: id, session_id: parent, provider: 'claude', data,
  });
  test('only the same still-pending confirmation is replaced', () => {
    noteSpawnConfirmationRequested(message('sc-1', 7, disclosure()));
    const fresh = disclosure({ billing_revision: 'b'.repeat(64) });
    expect(refreshSpawnConfirmationDisclosure('sc-1', 8, message('sc-1', 8, fresh))).toBe(false);
    expect(refreshSpawnConfirmationDisclosure('sc-2', 7, message('sc-2', 7, fresh))).toBe(false);
    expect(refreshSpawnConfirmationDisclosure('sc-1', 7, { ...message('sc-1', 7, fresh), type: 'other' })).toBe(false);
    expect(refreshSpawnConfirmationDisclosure('sc-1', 7, message('sc-1', 7, fresh))).toBe(true);
    expect(recordFromMessage(message('sc-1', 7, fresh)).billing.revision).toBe('b'.repeat(64));
  });
});

describe('which Hub refusals the dialog shows as a billing refusal', () => {
  test('only billing refusals carry their fixed text; other refusals keep the old handling', () => {
    const fixed = 'headless billing profile is unavailable';
    expect(billingRefusalDetail(400, { error: 'bad_request', detail: fixed })).toBe(fixed);
    expect(billingRefusalDetail(400, { error: 'bad_request', detail: 'API key billing cannot select a subscription profile' }))
      .toBe('API key billing cannot select a subscription profile');
    expect(billingRefusalDetail(403, {
      error: 'human_confirmation_required', detail: "API key billing needs an approval in the dashboard's spawn confirmation dialog",
    })).toBe("API key billing needs an approval in the dashboard's spawn confirmation dialog");
    // A model mismatch or any other refusal is not about billing.
    expect(billingRefusalDetail(400, { error: 'bad_request', detail: 'invalid model value' })).toBeUndefined();
    expect(billingRefusalDetail(403, { error: 'spawn_refused', detail: 'headless billing profile is unavailable' })).toBeUndefined();
    expect(billingRefusalDetail(500, { error: 'internal', detail: fixed })).toBeUndefined();
    expect(billingRefusalDetail(400, {})).toBeUndefined();
    expect(billingRefusalDetail(400, { detail: `headless billing ${'x'.repeat(400)}` })?.length).toBe(300);
  });
});
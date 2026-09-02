import { describe, expect, test } from 'bun:test';
import {
  DEFAULT_APPROVAL_FORM_SETTINGS,
  mergeApprovalSettings,
  defaultApprovalSettings,
  isApprovalSettingsMemoryEnabled,
  restoreApprovalSettings,
} from '../src/app/spawn-approval-memory.ts';

describe('spawn approval memory', () => {
  test('defaults to remembering and only an explicit false opts out', () => {
    expect(isApprovalSettingsMemoryEnabled({})).toBe(true);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: 'true' })).toBe(true);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: 'false' })).toBe(false);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: false })).toBe(true);
  });

  test('restores the existing approval values when the flag is omitted', () => {
    expect(restoreApprovalSettings('codex', {
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    })).toEqual({
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    });
    expect(restoreApprovalSettings('claude', {
      permission_mode: 'bypassPermissions',
    })).toEqual({ permission_mode: 'bypassPermissions' });
  });

  test('uses safe defaults when remembering is disabled or values are invalid', () => {
    expect(restoreApprovalSettings('codex', {
      remember_approval_settings: 'false',
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    })).toEqual(defaultApprovalSettings('codex'));
    expect(restoreApprovalSettings('codex', {
      sandbox: 'unrestricted',
      ask_for_approval: 'later',
    })).toEqual(defaultApprovalSettings('codex'));
    expect(restoreApprovalSettings('copilot', { permission_mode: 'plan' }))
      .toEqual({ permission_mode: DEFAULT_APPROVAL_FORM_SETTINGS.permission_mode });
    expect(restoreApprovalSettings('command-code', { permission_mode: 'auto' }))
      .toEqual({ permission_mode: DEFAULT_APPROVAL_FORM_SETTINGS.permission_mode });
    expect(restoreApprovalSettings('opencode', { opencode_permission_mode: 'unexpected' }))
      .toEqual({ opencode_permission_mode: DEFAULT_APPROVAL_FORM_SETTINGS.opencode_permission_mode });
  });

  test('keeps unrelated defaults and stores only the selected provider values', () => {
    const defaults = {
      provider: 'codex',
      cwd: 'D:\\work',
      model: '',
      other_setting: 'keep',
      permission_mode: 'bypassPermissions',
      sandbox: 'read-only',
      ask_for_approval: 'untrusted',
      opencode_permission_mode: 'bypassPermissions',
      risk_confirmed: 'true',
    };
    expect(mergeApprovalSettings(defaults, 'codex', {
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
      risk_confirmed: 'true',
    }, true)).toEqual({
      provider: 'codex',
      cwd: 'D:\\work',
      model: '',
      other_setting: 'keep',
      remember_approval_settings: 'true',
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    });
  });

  test('removes all remembered approval values when switched off', () => {
    expect(mergeApprovalSettings({
      provider: 'codex',
      cwd: 'D:\\work',
      permission_mode: 'bypassPermissions',
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
      opencode_permission_mode: 'bypassPermissions',
    }, 'codex', {}, false)).toEqual({
      provider: 'codex',
      cwd: 'D:\\work',
      remember_approval_settings: 'false',
    });
  });

  test('stores each provider family in its own approval keys', () => {
    expect(mergeApprovalSettings({}, 'claude', { permission_mode: 'plan' }, true))
      .toEqual({ remember_approval_settings: 'true', permission_mode: 'plan' });
    expect(mergeApprovalSettings({}, 'opencode', { opencode_permission_mode: 'bypassPermissions' }, true))
      .toEqual({ remember_approval_settings: 'true', opencode_permission_mode: 'bypassPermissions' });
    expect(mergeApprovalSettings({}, 'shell', { permission_mode: 'bypassPermissions' }, true))
      .toEqual({ remember_approval_settings: 'true' });
  });
});

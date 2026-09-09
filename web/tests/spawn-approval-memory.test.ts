import { describe, expect, test } from 'bun:test';
import {
  DEFAULT_APPROVAL_FORM_SETTINGS,
  mergeApprovalSettings,
  defaultApprovalSettings,
  hasProviderBooleanSetting,
  isApprovalSettingsMemoryEnabled,
  mergeProviderBooleanSetting,
  providerKey,
  restoreApprovalSettings,
  restoreProviderBooleanSetting,
  selectedApprovalSettings,
} from '../src/app/spawn-approval-memory.ts';

describe('spawn approval memory', () => {
  test('defaults to remembering and only an explicit false opts out', () => {
    expect(isApprovalSettingsMemoryEnabled({})).toBe(true);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: 'true' })).toBe(true);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: 'false' })).toBe(false);
    expect(isApprovalSettingsMemoryEnabled({ remember_approval_settings: false })).toBe(true);
  });

  test('providerKey joins the setting name and provider with an underscore', () => {
    expect(providerKey('permission_mode', 'claude')).toBe('permission_mode_claude');
    expect(providerKey('sandbox', 'codex')).toBe('sandbox_codex');
    expect(providerKey('isolate_worktree', 'cursor-agent')).toBe('isolate_worktree_cursor-agent');
  });

  test('restores from the legacy flat key when no namespaced key exists yet', () => {
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

  test('restores from the namespaced key when it exists, ignoring the stale flat key', () => {
    expect(restoreApprovalSettings('claude', {
      permission_mode_claude: 'bypassPermissions',
      // 移行前の残骸。名前空間キーがある以上こちらは無視される。
      permission_mode: 'plan',
    })).toEqual({ permission_mode: 'bypassPermissions' });
  });

  test('maps the current form values to the active provider', () => {
    expect(selectedApprovalSettings('codex', {
      permission_mode: 'bypassPermissions',
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    })).toEqual({
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    });
    expect(selectedApprovalSettings('claude', {
      permission_mode: 'plan',
      sandbox: 'danger-full-access',
    })).toEqual({ permission_mode: 'plan' });
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

  test('keeps unrelated defaults, drops the legacy flat keys, and stores only the namespaced provider values', () => {
    const defaults = {
      provider: 'codex',
      cwd: 'D:\\work',
      model: '',
      other_setting: 'keep',
      // 旧フラットキー（移行対象）。remember の値に関わらず常に捨てられる。
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
      sandbox_codex: 'danger-full-access',
      ask_for_approval_codex: 'never',
    });
  });

  test('turning memory off clears the current provider and always drops legacy flat keys', () => {
    expect(mergeApprovalSettings({
      provider: 'codex',
      cwd: 'D:\\work',
      // 旧フラットキー（常に捨てる）。
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

  // C3 (plan_spawn-form-per-provider-memory.md): 記憶 OFF の意味を広げる。
  // 以前（C1/C2）は OFF にした provider の許可設定キーしか消えなかったが、
  // C3 では許可設定・subscription・worktree・委譲の記憶を全 provider ぶん一括で消す。
  test('turning memory off wipes subscription / permission / worktree / delegation memory for every provider (C3 完了条件 2)', () => {
    expect(mergeApprovalSettings({
      provider: 'codex',
      cwd: 'D:\\work',
      other_setting: 'keep',
      permission_mode_claude: 'bypassPermissions',
      sandbox_codex: 'danger-full-access',
      ask_for_approval_codex: 'never',
      opencode_permission_mode_opencode: 'bypassPermissions',
      subscription_claude: 'plus-hiroshi',
      subscription_codex: 'auto',
      isolate_worktree_claude: 'true',
      delegation_grok: 'true',
    }, 'codex', {}, false)).toEqual({
      provider: 'codex',
      cwd: 'D:\\work',
      other_setting: 'keep',
      remember_approval_settings: 'false',
    });
  });

  test('while memory is off, restoring any provider falls back to that provider default, not stale memory (C3 完了条件 3)', () => {
    const offWithStaleMemory = {
      remember_approval_settings: 'false',
      permission_mode_claude: 'bypassPermissions',
      subscription_claude: 'plus-hiroshi',
      isolate_worktree_claude: 'true',
      delegation_claude: 'true',
    };
    expect(restoreApprovalSettings('claude', offWithStaleMemory)).toEqual(defaultApprovalSettings('claude'));
    expect(restoreProviderBooleanSetting('isolate_worktree', 'claude', offWithStaleMemory)).toBe(false);
    expect(restoreProviderBooleanSetting('delegation', 'claude', offWithStaleMemory)).toBe(false);
    expect(hasProviderBooleanSetting('isolate_worktree', 'claude', offWithStaleMemory)).toBe(false);
  });

  test('turning memory back on starts from nothing: wiped values do not come back (C3 完了条件 4)', () => {
    const beforeOff = mergeApprovalSettings({}, 'claude', { permission_mode: 'bypassPermissions' }, true);
    const afterOff = mergeApprovalSettings(beforeOff, 'claude', {}, false);
    expect(afterOff).toEqual({ remember_approval_settings: 'false' });

    // ON に戻した直後（まだ何も選び直していない）は、消した値が復活せず provider 既定のまま。
    const afterBackOn = mergeApprovalSettings(afterOff, 'claude', {}, true);
    expect(afterBackOn).toEqual({ remember_approval_settings: 'true' });
    expect(restoreApprovalSettings('claude', afterBackOn)).toEqual(defaultApprovalSettings('claude'));
  });

  test('stores each provider family under its own namespaced approval keys', () => {
    expect(mergeApprovalSettings({}, 'claude', { permission_mode: 'plan' }, true))
      .toEqual({ remember_approval_settings: 'true', permission_mode_claude: 'plan' });
    expect(mergeApprovalSettings({}, 'codex', {
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    }, true)).toEqual({
      remember_approval_settings: 'true',
      sandbox_codex: 'danger-full-access',
      ask_for_approval_codex: 'never',
    });
    expect(mergeApprovalSettings({}, 'opencode', { opencode_permission_mode: 'bypassPermissions' }, true))
      .toEqual({ remember_approval_settings: 'true', opencode_permission_mode_opencode: 'bypassPermissions' });
    expect(mergeApprovalSettings({}, 'shell', { permission_mode: 'bypassPermissions' }, true))
      .toEqual({ remember_approval_settings: 'true' });
  });

  test('switching the provider restores that provider default, not the other provider value (C1 完了条件 1・2)', () => {
    const afterClaudeSave = mergeApprovalSettings({}, 'claude', { permission_mode: 'bypassPermissions' }, true);
    expect(afterClaudeSave).toEqual({
      remember_approval_settings: 'true',
      permission_mode_claude: 'bypassPermissions',
    });

    // 1: claude を保存した後、grok に切り替えると default が復元される。
    expect(restoreApprovalSettings('grok', afterClaudeSave)).toEqual(defaultApprovalSettings('grok'));

    // 2: grok を plan にして保存しても、claude の bypassPermissions は残る。
    const afterGrokSave = mergeApprovalSettings(afterClaudeSave, 'grok', { permission_mode: 'plan' }, true);
    expect(afterGrokSave).toEqual({
      remember_approval_settings: 'true',
      permission_mode_claude: 'bypassPermissions',
      permission_mode_grok: 'plan',
    });
    expect(restoreApprovalSettings('claude', afterGrokSave)).toEqual({ permission_mode: 'bypassPermissions' });
    expect(restoreApprovalSettings('grok', afterGrokSave)).toEqual({ permission_mode: 'plan' });
  });

  test('migrates the legacy flat key to the namespaced key on save (C1 完了条件 3)', () => {
    const legacyOnly = { permission_mode: 'bypassPermissions' };
    // 保存前: 旧フラットキーだけでも claude の値として復元される。
    expect(restoreApprovalSettings('claude', legacyOnly)).toEqual({ permission_mode: 'bypassPermissions' });

    // 保存後: 名前空間キーへ移り、旧フラットキーは消える。
    const migrated = mergeApprovalSettings(legacyOnly, 'claude', { permission_mode: 'bypassPermissions' }, true);
    expect(migrated).toEqual({
      remember_approval_settings: 'true',
      permission_mode_claude: 'bypassPermissions',
    });
    expect(Object.prototype.hasOwnProperty.call(migrated, 'permission_mode')).toBe(false);
  });

  test('codex sandbox/ask_for_approval keys stay separate from claude permission_mode (C1 完了条件 4)', () => {
    const afterClaudeSave = mergeApprovalSettings({}, 'claude', { permission_mode: 'bypassPermissions' }, true);
    const afterCodexSave = mergeApprovalSettings(afterClaudeSave, 'codex', {
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    }, true);
    expect(afterCodexSave).toEqual({
      remember_approval_settings: 'true',
      permission_mode_claude: 'bypassPermissions',
      sandbox_codex: 'danger-full-access',
      ask_for_approval_codex: 'never',
    });
    // codex を保存しても claude の記憶は消えない。
    expect(restoreApprovalSettings('claude', afterCodexSave)).toEqual({ permission_mode: 'bypassPermissions' });
    expect(restoreApprovalSettings('codex', afterCodexSave)).toEqual({
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    });
  });
});

// C2 (plan_spawn-form-per-provider-memory.md): worktree 隔離・委譲チェックボックスの
// provider 別記憶。C1 の providerKey をそのまま流用し、許可設定と同じ移行順序
// （名前空間キー → 旧フラットキー → 未記憶）で読み書きする。
describe('spawn provider boolean memory (worktree / delegation, C2)', () => {
  test('providerKey builds the same namespaced key shape used by worktree / delegation', () => {
    expect(providerKey('isolate_worktree', 'claude')).toBe('isolate_worktree_claude');
    expect(providerKey('delegation', 'codex')).toBe('delegation_codex');
  });

  test('no memory yet: hasProviderBooleanSetting is false and restore falls back to unchecked', () => {
    expect(hasProviderBooleanSetting('isolate_worktree', 'codex', {})).toBe(false);
    expect(restoreProviderBooleanSetting('isolate_worktree', 'codex', {})).toBe(false);
  });

  test('restores from the legacy flat key when no namespaced key exists yet', () => {
    expect(hasProviderBooleanSetting('isolate_worktree', 'claude', { isolate_worktree: 'true' })).toBe(true);
    expect(restoreProviderBooleanSetting('isolate_worktree', 'claude', { isolate_worktree: 'true' })).toBe(true);
  });

  test('restores from the namespaced key when it exists, ignoring the stale flat key', () => {
    expect(restoreProviderBooleanSetting('isolate_worktree', 'claude', {
      isolate_worktree_claude: 'false',
      // 移行前の残骸。名前空間キーがある以上こちらは無視される。
      isolate_worktree: 'true',
    })).toBe(false);
  });

  test('merging keeps unrelated defaults, drops the legacy flat keys, and stores only the namespaced values', () => {
    const defaults = {
      provider: 'claude',
      cwd: 'D:\\work',
      permission_mode_claude: 'bypassPermissions',
      other_setting: 'keep',
      // 旧フラットキー（移行対象）。常に捨てられる。
      isolate_worktree: 'true',
      delegation: 'false',
    };
    expect(mergeProviderBooleanSetting(defaults, 'claude', {
      isolate_worktree: true,
      delegation: false,
    })).toEqual({
      provider: 'claude',
      cwd: 'D:\\work',
      permission_mode_claude: 'bypassPermissions',
      other_setting: 'keep',
      isolate_worktree_claude: 'true',
      delegation_claude: 'false',
    });
  });

  test('merging only one name leaves the other name for the same provider untouched', () => {
    expect(mergeProviderBooleanSetting({
      isolate_worktree_claude: 'true',
      delegation_claude: 'true',
    }, 'claude', { isolate_worktree: false })).toEqual({
      isolate_worktree_claude: 'false',
      delegation_claude: 'true',
    });
  });

  test('checking worktree for claude then switching to codex restores unchecked, switching back keeps it (C2 完了条件 1)', () => {
    const afterClaudeCheck = mergeProviderBooleanSetting({}, 'claude', { isolate_worktree: true });
    expect(afterClaudeCheck).toEqual({ isolate_worktree_claude: 'true' });
    expect(restoreProviderBooleanSetting('isolate_worktree', 'codex', afterClaudeCheck)).toBe(false);
    expect(restoreProviderBooleanSetting('isolate_worktree', 'claude', afterClaudeCheck)).toBe(true);
  });

  test('the same holds for the delegation checkbox (C2 完了条件 2)', () => {
    const afterClaudeCheck = mergeProviderBooleanSetting({}, 'claude', { delegation: true });
    expect(restoreProviderBooleanSetting('delegation', 'codex', afterClaudeCheck)).toBe(false);
    expect(restoreProviderBooleanSetting('delegation', 'claude', afterClaudeCheck)).toBe(true);
  });

  test('a provider with no memory has none to send, and spawning another provider does not drop it (C2 完了条件 5・6)', () => {
    // claude で worktree ON と許可設定を記憶。
    const afterClaudeWorktree = mergeProviderBooleanSetting({}, 'claude', { isolate_worktree: true });
    const afterClaudeApproval = mergeApprovalSettings(
      afterClaudeWorktree, 'claude', { permission_mode: 'bypassPermissions' }, true,
    );
    // codex には isolate_worktree の記憶が無い → spawnSession はキー自体を送信に含めない
    // （hasProviderBooleanSetting が false のときは bodyObj.isolate_worktree を省略する）。
    expect(hasProviderBooleanSetting('isolate_worktree', 'codex', afterClaudeApproval)).toBe(false);

    // codex で起動して許可設定を保存しても、claude の許可設定・worktree の記憶は消えない。
    const afterCodexApproval = mergeApprovalSettings(afterClaudeApproval, 'codex', {
      sandbox: 'danger-full-access',
      ask_for_approval: 'never',
    }, true);
    expect(restoreApprovalSettings('claude', afterCodexApproval)).toEqual({ permission_mode: 'bypassPermissions' });
    expect(restoreProviderBooleanSetting('isolate_worktree', 'claude', afterCodexApproval)).toBe(true);
  });
});

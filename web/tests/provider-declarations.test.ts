import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  cacheProviderDeclarations,
  clearProviderDeclarations,
  declaredProviderIds,
  providerBadgeLetter,
  providerContextLimit,
  providerFullFrameCursor,
  providerIgnoresLineClear,
  providerInterruptKey,
  providerIsAcp,
  providerIsBuiltin,
  providerLongLabel,
  providerPermissionChoices,
  providerShortLabel,
  providerSlowPaste,
  providerSortOrder,
  providerUsageUrl,
} from '../src/app/provider-declarations';
import { BUILTIN_DECLARATIONS_SEED, seedBuiltinProviderDeclarations } from '../src/app/provider-declarations-seed';
import { orchestrationCliOptions } from '../src/app/orchestration-roles';
import { isRegisteredChildProvider } from '../src/app/spawn-confirm-store';
import { defaultApprovalSettings, selectedApprovalSettings } from '../src/app/spawn-approval-memory';
import { shouldBypassCursorHideFilterForProvider } from '../src/app/cursor-hide-filter';

const BUILTIN_IDS = ['claude', 'codex', 'copilot', 'cursor-agent', 'opencode', 'grok', 'command-code'];

// 利用者が登録した AI CLI の宣言（Hub が同じ形で送る）。
const REGISTERED = {
  id: 'synthetic-cli',
  enabled: true,
  builtin: false,
  connection: 'pty',
  presentation: { letter: 'S', order: 40, label: 'Synthetic', long_label: 'Synthetic CLI Tool' },
  links: { usage_url: 'https://example.com/usage' },
  permissions: { choices: ['default', 'auto'] },
  input: { slow_paste: true, interrupt_key: 'ctrl-c', full_frame_cursor: true, no_line_clear: true },
  context_limit: 123_000,
};

beforeEach(() => seedBuiltinProviderDeclarations());
afterEach(() => clearProviderDeclarations());

describe('the web copy of the built-in declarations', () => {
  test('equals rust/assets/provider/declarations.json (the source of the values)', () => {
    const json = JSON.parse(
      readFileSync(new URL('../../rust/assets/provider/declarations.json', import.meta.url), 'utf8'),
    ) as Record<string, Record<string, unknown>>;
    expect(BUILTIN_DECLARATIONS_SEED.map((entry) => entry.id)).toEqual(Object.keys(json));
    for (const entry of BUILTIN_DECLARATIONS_SEED) {
      const source = json[entry.id];
      for (const key of ['presentation', 'slash_commands', 'permissions', 'links', 'input', 'routines']) {
        expect((entry as Record<string, unknown>)[key]).toEqual(source[key]);
      }
    }
  });
});

describe('what the screen used to hard-code, now read from the declarations', () => {
  test('the badge letter is one value for every screen (opencode is O in all of them)', () => {
    expect(BUILTIN_IDS.map((id) => providerBadgeLetter(id))).toEqual(['C', 'X', 'P', 'r', 'O', 'G', 'M']);
    // The chat avatar used to show P for opencode and A for copilot.
    expect(providerBadgeLetter('opencode')).toBe('O');
    expect(providerBadgeLetter('copilot')).toBe('P');
  });

  test('short and long names', () => {
    expect(BUILTIN_IDS.map((id) => providerShortLabel(id))).toEqual([
      'Claude', 'Codex', 'Copilot', 'Cursor Agent', 'OpenCode', 'Grok Build', 'Command Code',
    ]);
    expect(BUILTIN_IDS.map((id) => providerLongLabel(id))).toEqual([
      'Claude Code', 'Codex CLI', 'GitHub Copilot', 'Cursor Agent', 'OpenCode', 'Grok Build', 'Command Code',
    ]);
  });

  test('the order of the session list: claude, codex, copilot, opencode, cursor-agent, grok, command-code', () => {
    expect(declaredProviderIds()).toEqual([
      'claude', 'codex', 'copilot', 'opencode', 'cursor-agent', 'grok', 'command-code',
    ]);
    // Hub order (the order of the orchestration role list) is kept on request.
    expect(declaredProviderIds({ sort: 'hub' })).toEqual(BUILTIN_IDS);
    expect(providerSortOrder('claude') < providerSortOrder('codex')).toBe(true);
    expect(providerSortOrder('unknown')).toBe(99);
  });

  test('usage links', () => {
    expect(providerUsageUrl('claude')).toBe('https://claude.ai/settings/usage');
    expect(providerUsageUrl('codex')).toBe('https://chatgpt.com/codex/cloud/settings/analytics#usage');
    expect(providerUsageUrl('grok')).toBe('https://grok.com/?_s=usage');
    expect(providerUsageUrl('unknown')).toBe('');
  });

  test('permission modes, input habits', () => {
    const full = ['default', 'plan', 'acceptEdits', 'auto', 'bypassPermissions'];
    expect(providerPermissionChoices('claude')).toEqual(full);
    expect(providerPermissionChoices('grok')).toEqual(full);
    expect(providerPermissionChoices('copilot')).toEqual(['default', 'auto', 'bypassPermissions']);
    expect(providerPermissionChoices('command-code')).toEqual(['default', 'plan', 'acceptEdits', 'bypassPermissions']);
    expect(providerPermissionChoices('codex')).toEqual([]);
    expect(BUILTIN_IDS.filter((id) => providerSlowPaste(id))).toEqual(['codex', 'opencode']);
    expect(BUILTIN_IDS.filter((id) => providerInterruptKey(id) === 'ctrl-c')).toEqual(['grok']);
    expect(BUILTIN_IDS.filter((id) => providerFullFrameCursor(id))).toEqual(['codex', 'grok']);
    expect(BUILTIN_IDS.filter((id) => providerIgnoresLineClear(id))).toEqual(['codex']);
    expect(shouldBypassCursorHideFilterForProvider('grok')).toBe(true);
    expect(shouldBypassCursorHideFilterForProvider('claude')).toBe(false);
  });

  test('before the declarations arrive every answer is a neutral default', () => {
    clearProviderDeclarations();
    expect(providerBadgeLetter('cursor-agent')).toBe('C');
    expect(providerShortLabel('claude')).toBe('claude');
    expect(providerSortOrder('claude')).toBe(99);
    expect(providerUsageUrl('claude')).toBe('');
    expect(providerPermissionChoices('claude')).toEqual([]);
    expect(providerIsBuiltin('claude')).toBe(false);
    expect(declaredProviderIds()).toEqual([]);
  });
});

describe('a registered AI CLI is handled by the same functions', () => {
  beforeEach(() => cacheProviderDeclarations([...BUILTIN_DECLARATIONS_SEED, REGISTERED]));

  test('badge, names, order, usage link, permissions and input habits come from its declaration', () => {
    expect(providerBadgeLetter('synthetic-cli')).toBe('S');
    expect(providerShortLabel('synthetic-cli')).toBe('Synthetic');
    expect(providerLongLabel('synthetic-cli')).toBe('Synthetic CLI Tool');
    expect(providerUsageUrl('synthetic-cli')).toBe('https://example.com/usage');
    expect(providerPermissionChoices('synthetic-cli')).toEqual(['default', 'auto']);
    expect(providerSlowPaste('synthetic-cli')).toBe(true);
    expect(providerInterruptKey('synthetic-cli')).toBe('ctrl-c');
    expect(providerFullFrameCursor('synthetic-cli')).toBe(true);
    expect(providerIgnoresLineClear('synthetic-cli')).toBe(true);
    expect(providerContextLimit('synthetic-cli')).toBe(123_000);
    expect(providerIsBuiltin('synthetic-cli')).toBe(false);
    expect(providerIsAcp('synthetic-cli')).toBe(false);
    // Sorted after the built-ins that come before 40, and after all of them here.
    expect(declaredProviderIds().at(-1)).toBe('synthetic-cli');
  });

  test('the permission mode it declares is remembered; one it does not offer is dropped', () => {
    expect(selectedApprovalSettings('synthetic-cli', { permission_mode: 'auto' })).toEqual({ permission_mode: 'auto' });
    expect(selectedApprovalSettings('synthetic-cli', { permission_mode: 'plan' })).toEqual({ permission_mode: 'default' });
    expect(defaultApprovalSettings('synthetic-cli')).toEqual({ permission_mode: 'default' });
  });

  test('it is a registered child provider, the built-ins are not, and it is not in the built-in role list', () => {
    expect(isRegisteredChildProvider('synthetic-cli')).toBe(true);
    expect(BUILTIN_IDS.some((id) => isRegisteredChildProvider(id))).toBe(false);
    const roles = orchestrationCliOptions().map((option) => option.value);
    expect(roles).toEqual(['', ...BUILTIN_IDS]);
    expect(orchestrationCliOptions().map((option) => option.label).slice(1, 3)).toEqual(['Claude Code', 'Codex CLI']);
  });

  test('an ACP provider is kept out of the automatic lists', () => {
    cacheProviderDeclarations([...BUILTIN_DECLARATIONS_SEED, { ...REGISTERED, id: 'by-hand', connection: 'acp' }]);
    expect(providerIsAcp('by-hand')).toBe(true);
    expect(declaredProviderIds({ includeAcp: false })).not.toContain('by-hand');
    expect(declaredProviderIds()).toContain('by-hand');
  });
});

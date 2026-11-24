import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { cacheProviderDeclarations, clearProviderDeclarations } from '../src/app/provider-declarations';
import { BUILTIN_DECLARATIONS_SEED } from '../src/app/provider-declarations-seed';
import { permissionLabelKeys, permissionLabelText } from '../src/app/spawn-permission-label';

const ja = JSON.parse(readFileSync(new URL('../src/i18n/ja.json', import.meta.url), 'utf8')) as Record<string, string>;
const translate = (key: string): string => ja[key] ?? key;

const registered = (id: string) => ({ id, enabled: true, builtin: false, connection: 'pty', permissions: { choices: ['default', 'auto', 'bypassPermissions'] } });

const cursorAgentId = BUILTIN_DECLARATIONS_SEED.map((entry) => entry.id).find((id) => id.startsWith('cursor'));

beforeEach(() => cacheProviderDeclarations([
  ...BUILTIN_DECLARATIONS_SEED,
  registered('cursor'),
  registered('cursor-custom'),
  registered('copilot-like'),
]));
afterEach(() => clearProviderDeclarations());

describe('permission mode wording per provider', () => {
  test('the built-in Cursor Agent keeps its own wording', () => {
    expect(cursorAgentId).toBeTruthy();
    expect(permissionLabelText('auto', cursorAgentId!, translate)).toBe(ja.spawn_permission_auto_cursor);
    expect(permissionLabelText('bypass', cursorAgentId!, translate)).toBe(ja.spawn_permission_bypass_cursor);
  });

  test('a registered provider whose id starts with cursor gets the common wording', () => {
    for (const id of ['cursor', 'cursor-custom']) {
      expect(permissionLabelKeys('auto', id)).toEqual(['spawn_permission_auto']);
      expect(permissionLabelText('auto', id, translate)).toBe(ja.spawn_permission_auto);
      expect(permissionLabelText('bypass', id, translate)).toBe(ja.spawn_permission_bypass);
    }
    expect(permissionLabelText('bypass', 'copilot-like', translate)).toBe(ja.spawn_permission_bypass);
  });

  test('other built-ins use their own wording when there is one, else the common one', () => {
    const ids = BUILTIN_DECLARATIONS_SEED.map((entry) => entry.id);
    for (const id of ids) {
      const own = ja[`spawn_permission_bypass_${id.replace(/-/g, '_')}`];
      const expected = own ?? (id === cursorAgentId ? ja.spawn_permission_bypass_cursor : ja.spawn_permission_bypass);
      expect(permissionLabelText('bypass', id, translate)).toBe(expected);
    }
  });

  test('before any declaration arrives nothing is treated as built-in', () => {
    clearProviderDeclarations();
    expect(permissionLabelKeys('bypass', cursorAgentId!)).toEqual(['spawn_permission_bypass']);
  });
});

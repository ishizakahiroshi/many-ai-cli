import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import {
  cacheProviderDeclarations,
  clearProviderDeclarations,
  permissionLabelKeys,
  permissionLabelText,
} from '../src/app/provider-declarations';
import { BUILTIN_DECLARATIONS_SEED } from '../src/app/provider-declarations-seed';

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

// 上の検査は provider-declarations.ts の関数だけを見る。起動パネルが自前の判定
// （id が cursor で始まるか）へ戻ると、それらが全部通ったまま登録型にも Cursor の文言が出る。
// 起動パネルのファイルは名前で決め打ちせず、permissionLabel を定義しているファイルとして探す。
describe('the spawn UI takes its permission wording from provider-declarations', () => {
  const appDir = new URL('../src/app/', import.meta.url);
  const appSources = readdirSync(appDir)
    .filter((name) => name.endsWith('.ts'))
    .map((name) => ({ name, text: readFileSync(new URL(name, appDir), 'utf8') }));
  const labelOwners = appSources.filter((file) => /function permissionLabel\(/.test(file.text));

  test('permissionLabel delegates to permissionLabelText', () => {
    expect(labelOwners.length).toBe(1);
    const source = labelOwners[0].text;
    const body = source.match(/function permissionLabel\([^)]*\)[^{]*\{([\s\S]*?)\n  \}/)?.[1];
    expect(body).toBeDefined();
    expect(body!).toMatch(/return permissionLabelText\(kind, p,/);
    expect(source).toMatch(/function permissionAutoLabel\([^)]*\)[^{]*\{\s*return permissionLabel\('auto', p\);/);
    expect(source).toMatch(/function permissionBypassLabel\([^)]*\)[^{]*\{\s*return permissionLabel\('bypass', p\);/);
  });

  test('no app file checks the Cursor id prefix on its own', () => {
    // 判定の持ち主は provider-declarations.ts だけ（同梱かどうかを先に確かめる）。
    // 一致した箇所だけを比べる（ファイル全文を失敗の表示に出さない）。
    const found = appSources.filter((file) => file.name !== 'provider-declarations.ts').flatMap((file) =>
      (file.text.match(/startsWith\(\s*['"`]cursor[^)]*\)/g) ?? []).map((hit) => `${file.name}: ${hit}`),
    );
    expect(found).toEqual([]);
  });
});

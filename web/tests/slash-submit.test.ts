import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
  cacheProviderDeclarations,
  clearProviderDeclarations,
} from '../src/app/provider-declarations';
import { seedBuiltinProviderDeclarations, BUILTIN_DECLARATIONS_SEED } from '../src/app/provider-declarations-seed';
import {
  buildBodySubmit,
  mayUseSlashFallbackList,
  providerSlashSubmit,
  slashCommandQuickSends,
} from '../src/app/slash-submit';

const BUILTIN_IDS = ['claude', 'codex', 'copilot', 'cursor-agent', 'opencode', 'grok', 'command-code'];
const PASTE_START = '\x1b[200~';
const PASTE_END = '\x1b[201~';

// 登録した AI CLI（宣言は Hub が同じ形で送る）。
const ONCE_CLI = { id: 'synthetic-once', enabled: true, builtin: false, connection: 'pty' };
const PASTE_CLI = { id: 'synthetic-paste', enabled: true, builtin: false, connection: 'pty', input: { slash_submit: 'paste' } };
const LISTED_CLI = {
  id: 'synthetic-listed',
  enabled: true,
  builtin: false,
  connection: 'pty',
  slash_commands: { items: [{ cmd: '/help', desc: 'help' }, { cmd: '/model', desc: 'model' }] },
};

beforeEach(() => {
  seedBuiltinProviderDeclarations();
  cacheProviderDeclarations([...BUILTIN_DECLARATIONS_SEED, ONCE_CLI, PASTE_CLI, LISTED_CLI]);
});
afterEach(() => clearProviderDeclarations());

describe('how a one-line slash command is submitted', () => {
  test('the built-in providers send it as text plus Enter in one chunk, as before', () => {
    for (const id of BUILTIN_IDS) {
      expect(providerSlashSubmit(id)).toBe('once');
      expect(buildBodySubmit(id, '/help')).toEqual({ textPart: '/help\r', deferEnter: false });
    }
  });

  test('a registered provider without the declaration keeps the one-chunk default', () => {
    expect(buildBodySubmit('synthetic-once', '/help')).toEqual({ textPart: '/help\r', deferEnter: false });
  });

  test('a provider declaring paste sends it as a bracketed paste and a separate deferred Enter', () => {
    expect(providerSlashSubmit('synthetic-paste')).toBe('paste');
    expect(buildBodySubmit('synthetic-paste', '/help')).toEqual({
      textPart: PASTE_START + '/help' + PASTE_END,
      deferEnter: true,
    });
  });

  test('the other key-like inputs and ordinary text are unchanged for every provider', () => {
    for (const id of [...BUILTIN_IDS, 'synthetic-once', 'synthetic-paste', 'shell']) {
      expect(buildBodySubmit(id, '1')).toEqual({ textPart: '1\r', deferEnter: false });
      expect(buildBodySubmit(id, '')).toEqual({ textPart: '\r', deferEnter: false });
      expect(buildBodySubmit(id, 'hello')).toEqual(
        id === 'shell'
          ? { textPart: 'hello\r', deferEnter: false }
          : { textPart: PASTE_START + 'hello' + PASTE_END, deferEnter: true },
      );
      expect(buildBodySubmit(id, 'a\nb').deferEnter).toBe(true);
    }
  });

  test('shell always sends a slash line raw', () => {
    expect(buildBodySubmit('shell', '/tmp')).toEqual({ textPart: '/tmp\r', deferEnter: false });
  });
});

describe('the slash candidates never come from another CLI', () => {
  test('only the built-in providers may fall back to the fixed list', () => {
    for (const id of BUILTIN_IDS) expect(mayUseSlashFallbackList(id)).toBe(true);
    expect(mayUseSlashFallbackList('synthetic-once')).toBe(false);
    expect(mayUseSlashFallbackList('synthetic-listed')).toBe(false);
    expect(mayUseSlashFallbackList('unknown-id')).toBe(false);
  });

  test('before any declaration has arrived the behaviour stays as it was', () => {
    clearProviderDeclarations();
    expect(mayUseSlashFallbackList('claude')).toBe(true);
    expect(slashCommandQuickSends('claude', '/clear', [])).toBe(true);
  });
});

describe('the quick-send commands apply only where the provider has them', () => {
  const none: { cmd: string }[] = [];
  test('built-in providers quick-send /clear, /model and /models as before', () => {
    for (const id of BUILTIN_IDS) {
      for (const cmd of ['/clear', '/model', '/models']) expect(slashCommandQuickSends(id, cmd, none)).toBe(true);
    }
    expect(slashCommandQuickSends('claude', '/help', none)).toBe(false);
  });

  test('a registered provider quick-sends only a command that is in its own candidates', () => {
    const own = [{ cmd: '/help' }, { cmd: '/model' }];
    expect(slashCommandQuickSends('synthetic-listed', '/model', own)).toBe(true);
    expect(slashCommandQuickSends('synthetic-listed', '/clear', own)).toBe(false);
    expect(slashCommandQuickSends('synthetic-once', '/model', none)).toBe(false);
  });
});

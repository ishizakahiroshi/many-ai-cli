import assert from 'node:assert/strict';
import test from 'node:test';
import {
  aiCliGuideEntries,
  cliInstallStatuses,
  commandMissingIds,
  hasAvailableAiCli,
  isAiLaunchProvider,
  spawnLaunchBlockReason,
} from './cli-availability.js';

test('isAiLaunchProvider: shell と追加行は AI ではない', () => {
  assert.equal(isAiLaunchProvider('claude'), true);
  assert.equal(isAiLaunchProvider('my-cli'), true);
  assert.equal(isAiLaunchProvider('shell'), false);
  assert.equal(isAiLaunchProvider('__add-ai-provider__'), false);
  assert.equal(isAiLaunchProvider(''), false);
});

test('commandMissingIds: command_missing の field だけを集める', () => {
  const ids = commandMissingIds([
    { code: 'command_missing', field: 'claude' },
    { code: 'invalid_definition', field: 'codex' },
    { code: 'command_missing', field: 'grok' },
    { code: 'command_missing', field: '' },
  ]);
  assert.deepEqual([...ids].sort(), ['claude', 'grok']);
  assert.equal(commandMissingIds(null).size, 0);
  assert.equal(commandMissingIds(undefined).size, 0);
});

test('aiCliGuideEntries: 有効な AI だけ、表示名は provider 定義', () => {
  const entries = aiCliGuideEntries([
    { id: 'claude', display_name: 'Claude', enabled: true },
    { id: 'grok', display_name: 'Grok Build', enabled: true },
    { id: 'codex', display_name: 'Codex', enabled: false },
    { id: 'shell', display_name: 'Shell', enabled: true },
    { id: 'custom', display_name: '', enabled: true },
  ]);
  assert.deepEqual(entries, [
    { id: 'claude', displayName: 'Claude' },
    { id: 'grok', displayName: 'Grok Build' },
    { id: 'custom', displayName: 'custom' },
  ]);
});

test('hasAvailableAiCli: 有効な AI が 1 本でも PATH にあれば true', () => {
  const providers = [
    { id: 'claude', enabled: true },
    { id: 'grok', enabled: true },
    { id: 'shell', enabled: true },
  ];
  assert.equal(hasAvailableAiCli(providers, new Set(['claude', 'grok'])), false);
  assert.equal(hasAvailableAiCli(providers, new Set(['claude'])), true);
  assert.equal(hasAvailableAiCli(providers, new Set()), true);
});

test('hasAvailableAiCli: 有効な AI が 0 本なら案内を出さない', () => {
  assert.equal(hasAvailableAiCli([{ id: 'claude', enabled: false }, { id: 'shell', enabled: true }], new Set(['claude'])), true);
  assert.equal(hasAvailableAiCli([], new Set(['claude'])), true);
});

test('cliInstallStatuses: 導入済み・未導入+リンクあり・未導入+リンク無し・無効 provider は出ない・http は捨てる', () => {
  const providers = [
    { id: 'claude', display_name: 'Claude', enabled: true },
    { id: 'grok', display_name: 'Grok Build', enabled: true },
    { id: 'command-code', display_name: 'Command Code', enabled: true },
    { id: 'codex', display_name: 'Codex', enabled: false },
    { id: 'shell', display_name: 'Shell', enabled: true },
  ];
  const missingIds = new Set(['grok', 'command-code']);
  const installLinks = {
    grok: 'https://example.com/grok-install',
    // command-code はリンク無し
    codex: 'https://example.com/codex-install', // 無効 provider なので出ない
    claude: 'http://example.com/insecure', // http は捨てる
  };
  const statuses = cliInstallStatuses(providers, missingIds, installLinks);
  assert.deepEqual(statuses, [
    { id: 'claude', displayName: 'Claude', installed: true },
    { id: 'grok', displayName: 'Grok Build', installed: false, installUrl: 'https://example.com/grok-install' },
    { id: 'command-code', displayName: 'Command Code', installed: false },
  ]);
});

test('cliInstallStatuses: installLinks が null/undefined でも落ちない', () => {
  const providers = [{ id: 'claude', display_name: 'Claude', enabled: true }];
  assert.deepEqual(cliInstallStatuses(providers, new Set(['claude']), null), [
    { id: 'claude', displayName: 'Claude', installed: false },
  ]);
  assert.deepEqual(cliInstallStatuses(providers, new Set(), undefined), [
    { id: 'claude', displayName: 'Claude', installed: true },
  ]);
});

test('spawnLaunchBlockReason: cwd 欠けが先、次に CLI 未検出', () => {
  assert.equal(spawnLaunchBlockReason({
    cwd: '',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: true,
  }), 'cwd-empty');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: true,
    providerId: 'claude',
    commandMissing: true,
  }), 'cwd-missing');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: true,
  }), 'cli-missing');
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'shell',
    commandMissing: true,
  }), null);
  assert.equal(spawnLaunchBlockReason({
    cwd: 'C:\\dev\\app',
    cwdMissing: false,
    providerId: 'claude',
    commandMissing: false,
  }), null);
});

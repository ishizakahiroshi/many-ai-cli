import assert from 'node:assert/strict';
import test from 'node:test';
import {
  BUILTIN_PROVIDER_CAPABILITIES,
  DEFAULT_PROVIDER_CAPABILITIES,
  cacheProviderCapabilities,
  clearProviderCapabilitiesCache,
  hasProviderCapability,
  providerCapabilitiesFor,
  setProviderCapability,
} from './provider-capabilities.js';

test('providerCapabilitiesFor: 組み込みプロバイダの既定 capability を返す', () => {
  assert.equal(hasProviderCapability('claude', 'transcript'), true);
  assert.equal(hasProviderCapability('codex', 'approval'), true);
  assert.equal(hasProviderCapability('copilot', 'transcript'), false);
  assert.equal(hasProviderCapability('copilot', 'approval'), true);
  assert.equal(hasProviderCapability('command-code', 'transcript'), true);
  assert.equal(hasProviderCapability('command-code', 'usage'), false);
  assert.equal(hasProviderCapability('shell', 'approval'), false);
  assert.equal(hasProviderCapability('shell', 'launch'), true);
});

test('providerCapabilitiesFor: 未知のプロバイダは DEFAULT_PROVIDER_CAPABILITIES を返す', () => {
  assert.equal(hasProviderCapability('unknown-agent', 'launch'), true);
  assert.equal(hasProviderCapability('unknown-agent', 'approval'), false);
  assert.equal(hasProviderCapability('unknown-agent', 'transcript'), false);
  assert.equal(hasProviderCapability('unknown-agent', 'usage'), false);
});

test('cacheProviderCapabilities: API 取得値でキャッシュを更新できる', () => {
  clearProviderCapabilitiesCache();
  try {
    cacheProviderCapabilities([
      {
        id: 'custom-cli',
        capabilities: {
          launch: true,
          models: true,
          effort: false,
          headless: false,
          approval: true,
          transcript: true,
          usage: false,
          subscription: false,
          permissions: true,
        },
      },
    ]);
    assert.equal(hasProviderCapability('custom-cli', 'approval'), true);
    assert.equal(hasProviderCapability('custom-cli', 'transcript'), true);
    assert.equal(hasProviderCapability('custom-cli', 'usage'), false);
  } finally {
    clearProviderCapabilitiesCache();
  }
});

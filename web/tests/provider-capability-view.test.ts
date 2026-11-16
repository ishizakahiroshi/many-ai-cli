import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  capabilityLabelKey,
  capabilityLines,
  capabilityStateKey,
  capabilityUnsupportedReasonKey,
  capabilityView,
} from '../src/app/provider-manager-view.ts';

describe('登録画面の「できること／できないこと」', () => {
  test('Hub の答えを、使える → 使えない → 未対応の順に並べ、使える数を数える', () => {
    const view = capabilityView([
      { key: 'usage_meter', state: 'unsupported', detail: 'x' },
      { key: 'headless', state: 'unavailable', detail: 'no headless' },
      { key: 'launch', state: 'available' },
      { key: 'slash_commands', state: 'available' },
    ]);
    expect(view.total).toBe(4);
    expect(view.available).toBe(2);
    expect(view.lines.map((line) => line.key)).toEqual(['launch', 'slash_commands', 'headless', 'usage_meter']);
  });

  test('形の違う行は捨て、知らない state は「使えない」として出す', () => {
    const lines = capabilityLines([null, 'x', { key: 7 }, { key: 'Bad Key', state: 'available' }, { key: 'login', state: 'maybe' }]);
    expect(lines).toEqual([{ key: 'login', state: 'unavailable', detail: '' }]);
    expect(capabilityLines(undefined)).toEqual([]);
    expect(capabilityView({}).total).toBe(0);
  });

  test('Hub が返すどの項目にも、3 言語の表示名があり、未対応の項目には理由がある', () => {
    const keys = [
      'launch', 'headless', 'models', 'model_flag', 'effort', 'approval', 'transcript', 'chat_view',
      'slash_commands', 'permissions', 'usage_link', 'login', 'automation',
      'usage_meter', 'subscription_profiles', 'subagent_tree',
    ];
    for (const lang of ['en', 'ja', 'vi']) {
      const table = JSON.parse(readFileSync(new URL(`../src/i18n/${lang}.json`, import.meta.url), 'utf8')) as Record<string, string>;
      for (const key of keys) expect(table[capabilityLabelKey(key)], `${lang} ${key}`).toBeTruthy();
      for (const state of ['available', 'unavailable', 'unsupported'] as const) {
        expect(table[capabilityStateKey(state)], `${lang} ${state}`).toBeTruthy();
      }
      for (const key of ['usage_meter', 'subscription_profiles', 'subagent_tree']) {
        expect(table[capabilityUnsupportedReasonKey(key)], `${lang} why ${key}`).toBeTruthy();
      }
      expect(table.settings_ai_providers_cap_summary).toContain('{available}');
    }
  });

  test('Rust の capability_report.rs が出す key が、上の一覧と同じ', () => {
    const source = readFileSync(new URL('../../rust/src/profile/capability_report.rs', import.meta.url), 'utf8');
    for (const key of ['launch', 'headless', 'models', 'model_flag', 'effort', 'approval', 'transcript', 'chat_view',
      'slash_commands', 'permissions', 'usage_link', 'login', 'automation',
      'usage_meter', 'subscription_profiles', 'subagent_tree']) {
      expect(source.includes(`"${key}"`), key).toBe(true);
    }
  });
});

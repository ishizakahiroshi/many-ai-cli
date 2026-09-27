import { describe, expect, test } from 'bun:test';
import { duplicateProviderDraft } from '../src/app/provider-manager-view.ts';

// plan_provider-registry-externalization-orchestration_c8_management-ui.md C6。
// id 採番のみを検査する純関数テスト。DOM やネットワークには触れない。

describe('duplicateProviderDraft', () => {
  test('空いている場合は "-copy" を使う', () => {
    const draft = duplicateProviderDraft({ id: 'synth-cli', display_name: 'Synth CLI' }, new Set());
    expect(draft.id).toBe('synth-cli-copy');
    expect(draft.displayName).toBe('Synth CLI');
  });

  test('"-copy" が使用済みなら "-copy-2" を使う', () => {
    const existingIds = new Set(['synth-cli', 'synth-cli-copy']);
    const draft = duplicateProviderDraft({ id: 'synth-cli', display_name: 'Synth CLI' }, existingIds);
    expect(draft.id).toBe('synth-cli-copy-2');
  });

  test('"-copy" と "-copy-2" が使用済みなら "-copy-3" を使う', () => {
    const existingIds = new Set(['synth-cli', 'synth-cli-copy', 'synth-cli-copy-2']);
    const draft = duplicateProviderDraft({ id: 'synth-cli', display_name: 'Synth CLI' }, existingIds);
    expect(draft.id).toBe('synth-cli-copy-3');
  });

  test('表示名が無い場合は id を表示名として使う', () => {
    const draft = duplicateProviderDraft({ id: 'synth-cli' }, new Set());
    expect(draft.displayName).toBe('synth-cli');
  });
});

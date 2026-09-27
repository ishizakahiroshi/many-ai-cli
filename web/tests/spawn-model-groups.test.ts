import { describe, expect, test } from 'bun:test';
import {
  compatibleModelOrEmpty,
  getCachedSpawnModelGroups,
  getModelGroupsForProvider,
  getSpawnModelGroupCacheGeneration,
  invalidateSpawnModelGroups,
  isModelCompatibleWithProvider,
  setCachedSpawnModelGroups,
  spawnModelGroupPickerLabel,
  type SpawnModelGroup,
} from '../src/app/spawn-model-groups.ts';

// bugfix_derive-dialog-model-provider-mismatch_2026-09-12.md C1。
// 実カタログ名は焼き付けない。グループの provider タグと絞り規則だけを固定する。

const groups: SpawnModelGroup[] = [
  { label: 'Anthropic', provider: 'claude', route: 'anthropic', models: [{ id: 'synth-claude-a', label: 'Claude A' }] },
  { label: 'OpenAI', provider: 'codex', route: 'openai', models: [{ id: 'synth-codex-a', label: 'Codex A' }] },
  { label: 'GitHub Copilot', provider: 'copilot', route: '', models: [{ id: 'synth-copilot-a', label: 'Copilot A' }] },
  { label: 'Cursor Agent', provider: 'cursor-agent', route: '', models: [{ id: 'synth-cursor-a', label: 'Cursor A' }] },
  { label: 'Grok Build', provider: 'grok', route: '', models: [{ id: 'synth-grok-a', label: 'Grok A' }] },
  { label: 'OpenCode', provider: 'opencode', route: '', models: [{ id: 'synth-opencode-a', label: 'OpenCode A' }] },
  { label: 'NVIDIA NIM', provider: 'opencode', route: 'nvidia-nim', hosted: true, trial: true, models: [{ id: 'nvidia/synthetic-model-a', label: 'Synthetic model A' }] },
  { label: 'Ollama Cloud', provider: '', route: 'ollama', models: [{ id: 'synth-ollama-cloud' }] },
  { label: 'Ollama Local', provider: '', route: 'ollama', models: [{ id: 'synth-ollama-local' }] },
  { label: 'LM Studio', provider: '', route: 'lm-studio', models: [{ id: 'synth-lmstudio-a' }] },
];

function labelsFor(provider: string): string[] {
  return getModelGroupsForProvider(groups, provider).map((g) => g.label);
}

describe('getModelGroupsForProvider', () => {
  test('claude sees Anthropic plus empty-provider local groups, not other CLIs', () => {
    expect(labelsFor('claude')).toEqual(['Anthropic', 'Ollama Cloud', 'Ollama Local', 'LM Studio']);
  });

  test('codex sees OpenAI plus empty-provider local groups, not Anthropic or Grok', () => {
    expect(labelsFor('codex')).toEqual(['OpenAI', 'Ollama Cloud', 'Ollama Local', 'LM Studio']);
  });

  test('grok sees only Grok Build', () => {
    expect(labelsFor('grok')).toEqual(['Grok Build']);
  });

  test('copilot sees only GitHub Copilot', () => {
    expect(labelsFor('copilot')).toEqual(['GitHub Copilot']);
  });

  test('cursor-agent sees only Cursor Agent', () => {
    expect(labelsFor('cursor-agent')).toEqual(['Cursor Agent']);
  });

  test('opencode sees OpenCode plus empty-provider local groups', () => {
    expect(labelsFor('opencode')).toEqual(['OpenCode', 'NVIDIA NIM', 'Ollama Cloud', 'Ollama Local', 'LM Studio']);
  });

  test('NVIDIA NIM is an OpenCode-only hosted trial group with its route', () => {
    const nim = getModelGroupsForProvider(groups, 'opencode').find((group) => group.label === 'NVIDIA NIM');
    expect(nim).toMatchObject({ provider: 'opencode', route: 'nvidia-nim', hosted: true, trial: true });
    expect(getModelGroupsForProvider(groups, 'codex').some((group) => group.label === 'NVIDIA NIM')).toBe(false);
    expect(getModelGroupsForProvider(groups, 'claude').some((group) => group.label === 'NVIDIA NIM')).toBe(false);
  });

  test('NVIDIA picker label carries localized hosted and trial badges', () => {
    const nim = groups.find((group) => group.label === 'NVIDIA NIM')!;
    expect(spawnModelGroupPickerLabel(nim, { hosted: 'Hosted', trial: 'Trial' })).toBe('NVIDIA NIM · Hosted / Trial');
    expect(spawnModelGroupPickerLabel(groups[0], { hosted: 'Hosted', trial: 'Trial' })).toBe('Anthropic');
  });

  test('command-code has no dedicated group so only empty-provider locals appear', () => {
    expect(labelsFor('command-code')).toEqual(['Ollama Cloud', 'Ollama Local', 'LM Studio']);
  });

  test('null groups yield an empty list rather than throwing', () => {
    expect(getModelGroupsForProvider(null, 'claude')).toEqual([]);
  });
});

describe('isModelCompatibleWithProvider', () => {
  test('a Grok-only id is not compatible with claude', () => {
    expect(isModelCompatibleWithProvider(groups, 'claude', 'synth-grok-a')).toBe(false);
  });

  test('an NVIDIA NIM model is compatible only with OpenCode', () => {
    expect(isModelCompatibleWithProvider(groups, 'opencode', 'nvidia/synthetic-model-a')).toBe(true);
    expect(isModelCompatibleWithProvider(groups, 'codex', 'nvidia/synthetic-model-a')).toBe(false);
  });

  test('an empty model is compatible with every CLI', () => {
    expect(isModelCompatibleWithProvider(groups, 'claude', '')).toBe(true);
    expect(isModelCompatibleWithProvider(groups, 'grok', '   ')).toBe(true);
  });

  test('an id in no group is compatible (typed-in unknown stays allowed)', () => {
    expect(isModelCompatibleWithProvider(groups, 'claude', 'synth-unlisted')).toBe(true);
  });

  test('an empty-provider local id is compatible with every CLI (including grok, which does not list it)', () => {
    // 表示は grok 専用枝で Ollama を出さないが、互換判定は provider 空を全 CLI に通す。
    // 現行 spawn-panel の分岐を写したので、ここで grok だけ落とす改変はしない。
    expect(isModelCompatibleWithProvider(groups, 'claude', 'synth-ollama-local')).toBe(true);
    expect(isModelCompatibleWithProvider(groups, 'grok', 'synth-ollama-local')).toBe(true);
  });

  test('unloaded groups (null) treat any model as compatible', () => {
    expect(isModelCompatibleWithProvider(null, 'claude', 'synth-grok-a')).toBe(true);
  });
});

describe('compatibleModelOrEmpty', () => {
  test('keeps a matching id and drops a dedicated-other-CLI id', () => {
    expect(compatibleModelOrEmpty(groups, 'claude', 'synth-claude-a')).toBe('synth-claude-a');
    expect(compatibleModelOrEmpty(groups, 'claude', 'synth-grok-a')).toBe('');
    expect(compatibleModelOrEmpty(groups, 'claude', '')).toBe('');
  });
});

test('invalidating model groups clears the cached catalog and advances its generation', () => {
  const generation = getSpawnModelGroupCacheGeneration();
  setCachedSpawnModelGroups(groups);
  expect(getCachedSpawnModelGroups()).toEqual(groups);
  invalidateSpawnModelGroups();
  expect(getCachedSpawnModelGroups()).toBeNull();
  expect(getSpawnModelGroupCacheGeneration()).toBe(generation + 1);
});

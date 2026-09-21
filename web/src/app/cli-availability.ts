// PATH 上に公式 CLI があるかどうか。空画面の案内と spawn の起動ボタンが
// 同じ判定を使う。表示名と実行ファイル名は provider 定義（id / display_name）
// から組み立て、案内専用の一覧は持たない。

export const SHELL_PROVIDER_ID = 'shell';
export const ADD_PROVIDER_OPTION_VALUE = '__add-ai-provider__';

export type CliGuideEntry = {
  id: string;
  displayName: string;
};

export type SpawnLaunchBlock = 'cwd-empty' | 'cwd-missing' | 'cli-missing' | null;

export function isAiLaunchProvider(id: string): boolean {
  return !!id && id !== SHELL_PROVIDER_ID && id !== ADD_PROVIDER_OPTION_VALUE;
}

export function commandMissingIds(diagnostics: { code?: string; field?: string }[] | null | undefined): Set<string> {
  const ids = new Set<string>();
  if (!diagnostics) return ids;
  for (const diagnostic of diagnostics) {
    if (diagnostic?.code !== 'command_missing') continue;
    const field = String(diagnostic.field || '').trim();
    if (field) ids.add(field);
  }
  return ids;
}

export function aiCliGuideEntries(providers: { id: string; display_name?: string; enabled?: boolean }[]): CliGuideEntry[] {
  return providers
    .filter((provider) => provider.enabled !== false && isAiLaunchProvider(provider.id))
    .map((provider) => ({
      id: provider.id,
      displayName: (provider.display_name && provider.display_name.trim()) || provider.id,
    }));
}

export function hasAvailableAiCli(
  providers: { id: string; enabled?: boolean }[],
  missingIds: Set<string>,
): boolean {
  const enabledAi = providers.filter((provider) => (
    provider.enabled !== false && isAiLaunchProvider(provider.id)
  ));
  // 有効な AI が 1 本も無い（全部オフ）ときは入れ方案内を出さない。
  if (enabledAi.length === 0) return true;
  return enabledAi.some((provider) => !missingIds.has(provider.id));
}

export function spawnLaunchBlockReason(input: {
  cwd: string;
  cwdMissing: boolean;
  providerId: string;
  commandMissing: boolean;
}): SpawnLaunchBlock {
  if (!input.cwd.trim()) return 'cwd-empty';
  if (input.cwdMissing) return 'cwd-missing';
  if (input.commandMissing && isAiLaunchProvider(input.providerId)) return 'cli-missing';
  return null;
}

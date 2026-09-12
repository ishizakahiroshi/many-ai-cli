// /api/models の groups を、選んだ CLI で絞る純関数。
//
// spawn-panel.ts の閉包に置いていたフィルタをここへ出した。新規セッション画面・
// 派生ダイアログ・spawn 確認が同じ規則を使う。DOM に触れない関数は bun:test から
// 直接 import できる（fillModelDatalist だけ datalist を書く）。
//
// grok / copilot / cursor-agent は専用グループだけ。claude / codex / opencode と
// 専用グループの無い CLI（command-code など）は、自分のグループに加えて
// provider 空の Ollama / LM Studio を出す。この分岐を「全部フォールバック」へ
// まとめない（bugfix_derive-dialog-model-provider-mismatch_2026-09-12.md C1）。

export interface SpawnModel {
  id: string;
  label?: string;
}

export interface SpawnModelGroup {
  label: string;
  provider?: string;
  route?: string;
  models: SpawnModel[];
}

let cachedGroups: SpawnModelGroup[] | null = null;
let inFlight: Promise<SpawnModelGroup[]> | null = null;

export function getCachedSpawnModelGroups(): SpawnModelGroup[] | null {
  return cachedGroups;
}

/** テストと fetch 完了時だけが書く。画面は loadSpawnModelGroups 経由。 */
export function setCachedSpawnModelGroups(groups: SpawnModelGroup[] | null): void {
  cachedGroups = Array.isArray(groups) ? groups : null;
}

export function groupHasModel(group: SpawnModelGroup | null | undefined, model: string): boolean {
  const m = (model || '').trim();
  if (!m || !group?.models) return false;
  return group.models.some((entry) => entry && entry.id === m);
}

export function getModelGroupsForProvider(
  groups: SpawnModelGroup[] | null | undefined,
  provider: string,
): SpawnModelGroup[] {
  if (!Array.isArray(groups)) return [];
  if (provider === 'copilot') {
    return groups.filter((g) => g && g.provider === 'copilot' && Array.isArray(g.models));
  }
  if (provider === 'cursor-agent') {
    return groups.filter((g) => g && g.provider === 'cursor-agent' && Array.isArray(g.models));
  }
  if (provider === 'grok') {
    return groups.filter((g) => g && g.provider === 'grok' && Array.isArray(g.models));
  }
  const filtered = groups.filter((g) => g && Array.isArray(g.models) && (!g.provider || g.provider === provider));
  filtered.sort((a, b) => {
    const rank = (g: SpawnModelGroup): number => {
      if (g.provider === provider) return 0;
      if (g.label === 'Ollama Cloud') return 1;
      if (g.label === 'Ollama Local') return 2;
      if (g.label === 'LM Studio') return 3;
      return 4;
    };
    return rank(a) - rank(b);
  });
  return filtered;
}

export function isModelCompatibleWithProvider(
  groups: SpawnModelGroup[] | null | undefined,
  provider: string,
  model: string,
): boolean {
  const m = (model || '').trim();
  if (!m || !Array.isArray(groups)) return true;
  let known = false;
  for (const g of groups) {
    if (!groupHasModel(g, m)) continue;
    known = true;
    if (!g.provider || g.provider === provider) return true;
  }
  return !known;
}

export function compatibleModelOrEmpty(
  groups: SpawnModelGroup[] | null | undefined,
  provider: string,
  model: string,
): string {
  const m = (model || '').trim();
  if (!m) return '';
  return isModelCompatibleWithProvider(groups, provider, m) ? m : '';
}

export function fillModelDatalist(
  datalist: HTMLDataListElement | null,
  groups: SpawnModelGroup[] | null | undefined,
  provider: string,
): void {
  if (!datalist) return;
  datalist.innerHTML = '';
  if (!Array.isArray(groups)) return;
  for (const g of getModelGroupsForProvider(groups, provider)) {
    for (const m of g.models) {
      if (!m?.id) continue;
      const opt = document.createElement('option');
      opt.value = m.id;
      const label = `[${g.label}] ${m.label || m.id}`;
      opt.setAttribute('label', label);
      opt.textContent = label;
      opt.dataset.route = g.route || '';
      datalist.appendChild(opt);
    }
  }
}

export async function loadSpawnModelGroups(token: string, force = false): Promise<SpawnModelGroup[]> {
  if (inFlight) return inFlight;
  if (!force && cachedGroups) return cachedGroups;
  const method = force ? 'POST' : 'GET';
  const p = (async () => {
    try {
      const res = await fetch(`/api/models?token=${token}`, { method });
      if (!res.ok) throw new Error('HTTP ' + res.status);
      const data = await res.json();
      cachedGroups = Array.isArray(data?.groups) ? data.groups : [];
      return cachedGroups;
    } finally {
      inFlight = null;
    }
  })();
  inFlight = p;
  return p;
}

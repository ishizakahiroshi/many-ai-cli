// provider-order.ts — provider の並び順（端末・ブラウザ単位の localStorage）の単一ソース。
// 「新しいセッション」の provider 一覧（spawn-panel.ts）と初回画面の導入状況一覧
// （zero-session-empty-state.ts）が同じ順番を読み書きする。片方で並べ替えたら
// PROVIDER_ORDER_CHANGED_EVENT で他方が描き直す。
//
// 保存値は「並べ替えたことのある id の列」で、全 provider を網羅するとは限らない
// （custom provider・Shell・追加用の擬似 option も混ざりうる）。一覧に無い id は
// 捨てずに持ち続け、別の画面で並べ替えたときも位置を保つ（mergeProviderSubsetOrder）。

export const STORAGE_PROVIDER_ORDER_KEY = 'ai_cli_hub_spawn_provider_order';
export const PROVIDER_ORDER_CHANGED_EVENT = 'provider-order-changed';

export function loadProviderOrder(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_PROVIDER_ORDER_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((value): value is string => typeof value === 'string');
  } catch (_) {
    return [];
  }
}

function notifyProviderOrderChanged(): void {
  if (typeof document === 'undefined') return;
  document.dispatchEvent(new CustomEvent(PROVIDER_ORDER_CHANGED_EVENT));
}

export function saveProviderOrder(values: string[]): void {
  try { localStorage.setItem(STORAGE_PROVIDER_ORDER_KEY, JSON.stringify(values)); } catch (_) { /* private mode 等は無視 */ }
  notifyProviderOrderChanged();
}

export function clearProviderOrder(): void {
  try { localStorage.removeItem(STORAGE_PROVIDER_ORDER_KEY); } catch (_) { /* noop */ }
  notifyProviderOrderChanged();
}

// sortByProviderOrder は items を保存順に並べる。保存順に無いものは元の順のまま後ろへ付ける。
export function sortByProviderOrder<T>(items: T[], idOf: (item: T) => string, order: string[] = loadProviderOrder()): T[] {
  const rank = new Map<string, number>();
  order.forEach((id, index) => { if (!rank.has(id)) rank.set(id, index); });
  return items
    .map((item, index) => ({ item, index, rank: rank.get(idOf(item)) }))
    .sort((a, b) => {
      if (a.rank !== undefined && b.rank !== undefined) return a.rank - b.rank;
      if (a.rank !== undefined) return -1;
      if (b.rank !== undefined) return 1;
      return a.index - b.index;
    })
    .map((entry) => entry.item);
}

// mergeProviderSubsetOrder は一覧の一部（例: 導入済みの行だけ）を並べ替えた結果を全体の順へ
// 書き戻す。全体 = 保存順 + 保存順に無い known（元の順）。subset に含まれる id が占めていた
// 位置へ、subset の新しい順で詰め直す。subset 以外の id の位置は動かさない。
export function mergeProviderSubsetOrder(saved: string[], known: string[], subset: string[]): string[] {
  const full: string[] = [];
  const seen = new Set<string>();
  for (const id of [...saved, ...known]) {
    if (!seen.has(id)) { full.push(id); seen.add(id); }
  }
  const inSubset = new Set(subset);
  const queue = subset.filter((id) => seen.has(id));
  for (const id of subset) {
    if (!seen.has(id)) full.push(id);
  }
  let next = 0;
  return full.map((id) => (inSubset.has(id) && next < queue.length ? queue[next++] : id));
}

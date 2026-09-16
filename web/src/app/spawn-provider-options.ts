// spawn-provider-options.ts — spawn provider セレクタへの custom provider
// <option> 注入/整理を、DOM から切り離した純粋な差分計算として持つ。
//
// spawn-panel.ts の injectCustomProviderOptions は追加しかしていなかった
// （C2 敵対レビュー: custom provider を Provider Manager から削除した直後、
// 何かの拍子にこの関数が再実行されるまで select に消えたはずの選択肢が
// 残った）。DOM 操作と絡めたままではテストしにくいので、削除・追加の
// 判定だけをここに切り出す。副作用（option の追加/削除・change イベント
// 発火）は呼び出し側が戻り値を見て行う。

export type SpawnProviderOptionEntry = { id: string; label: string };

export type SpawnProviderOptionReconciliation = {
  toRemove: string[];
  toAdd: SpawnProviderOptionEntry[];
  resetSelection: boolean;
};

export function reconcileSpawnProviderOptions(input: {
  // このセッションで自分が注入した option の id（built-in や legacy custom_providers
  // など、この関数が管理しない option は対象にしない）。
  injectedIds: string[];
  // 最新の一覧（provider API が失敗した回は呼び出し側が reconcileRemovals=false
  // にして渡すか、そもそも呼ばない）。
  freshEntries: SpawnProviderOptionEntry[];
  // 現在 <select> に存在する option の value 一覧（built-in・「+ Add AI CLI…」等含む）。
  existingOptionValues: string[];
  // 現在選択中の value。
  selectedValue: string;
  // false のときは削除を計算しない（legacy /api/info フォールバックで一覧を
  // 取れた回など、一時的な取得失敗を消滅と誤認しないための切り替え）。
  reconcileRemovals: boolean;
}): SpawnProviderOptionReconciliation {
  const freshIds = new Set(input.freshEntries.map((entry) => entry.id));
  const toRemove = input.reconcileRemovals
    ? input.injectedIds.filter((id) => !freshIds.has(id))
    : [];
  const removedSet = new Set(toRemove);
  const existing = new Set(input.existingOptionValues.filter((value) => !removedSet.has(value)));
  const toAdd = input.freshEntries.filter((entry) => entry.id && !existing.has(entry.id));
  return {
    toRemove,
    toAdd,
    resetSelection: removedSet.has(input.selectedValue),
  };
}

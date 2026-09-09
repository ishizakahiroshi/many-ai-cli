export const REMEMBER_APPROVAL_SETTINGS_KEY = 'remember_approval_settings';

export const APPROVAL_SETTING_KEYS = [
  'permission_mode',
  'sandbox',
  'ask_for_approval',
  'opencode_permission_mode',
] as const;

export type ApprovalSettingKey = (typeof APPROVAL_SETTING_KEYS)[number];
export type SpawnDefaults = Readonly<Record<string, unknown>>;
export type SpawnApprovalSettings = Partial<Record<ApprovalSettingKey, string>>;

export const DEFAULT_APPROVAL_FORM_SETTINGS = Object.freeze({
  permission_mode: 'default',
  sandbox: 'workspace-write',
  ask_for_approval: 'on-request',
  opencode_permission_mode: 'default',
});

const PERMISSION_MODE_VALUES: Readonly<Record<string, readonly string[]>> = {
  claude: ['default', 'plan', 'acceptEdits', 'auto', 'bypassPermissions'],
  grok: ['default', 'plan', 'acceptEdits', 'auto', 'bypassPermissions'],
  copilot: ['default', 'auto', 'bypassPermissions'],
  'cursor-agent': ['default', 'auto', 'bypassPermissions'],
  'command-code': ['default', 'plan', 'acceptEdits', 'bypassPermissions'],
};

const CODEX_SANDBOX_VALUES = ['workspace-write', 'read-only', 'danger-full-access'] as const;
const CODEX_APPROVAL_VALUES = ['on-request', 'untrusted', 'never'] as const;
const OPENCODE_PERMISSION_VALUES = ['default', 'bypassPermissions'] as const;

function normalizeValue(value: unknown, allowed: readonly string[], fallback: string): string {
  return typeof value === 'string' && allowed.includes(value) ? value : fallback;
}

/**
 * 記憶キーを provider 別の名前空間へ分けるための共通の組み立て方。
 * 既存の `subscription_<provider>` と同じ `<名前>_<provider>` 形式に揃える。
 * 許可設定に限らず（C2 の worktree/委譲でも）同じ形を使う前提のため、
 * `name` は ApprovalSettingKey に絞らず string を受け付ける。
 */
export function providerKey(name: string, provider: string): string {
  return `${name}_${provider}`;
}

/** その provider が実際に使う許可設定のキー名（codex だけ 2 つ）。 */
function approvalKeyNamesForProvider(provider: string): readonly ApprovalSettingKey[] {
  if (Object.prototype.hasOwnProperty.call(PERMISSION_MODE_VALUES, provider)) return ['permission_mode'];
  if (provider === 'codex') return ['sandbox', 'ask_for_approval'];
  if (provider === 'opencode') return ['opencode_permission_mode'];
  return [];
}

export function selectedApprovalSettings(provider: string, values: SpawnDefaults): SpawnApprovalSettings {
  const out: SpawnApprovalSettings = {};
  if (Object.prototype.hasOwnProperty.call(PERMISSION_MODE_VALUES, provider)) {
    if (Object.prototype.hasOwnProperty.call(values, 'permission_mode')) {
      const allowed = PERMISSION_MODE_VALUES[provider];
      out.permission_mode = normalizeValue(values.permission_mode, allowed, 'default');
    }
    return out;
  }
  if (provider === 'codex') {
    if (Object.prototype.hasOwnProperty.call(values, 'sandbox')) {
      out.sandbox = normalizeValue(values.sandbox, CODEX_SANDBOX_VALUES, 'workspace-write');
    }
    if (Object.prototype.hasOwnProperty.call(values, 'ask_for_approval')) {
      out.ask_for_approval = normalizeValue(values.ask_for_approval, CODEX_APPROVAL_VALUES, 'on-request');
    }
    return out;
  }
  if (provider === 'opencode') {
    if (Object.prototype.hasOwnProperty.call(values, 'opencode_permission_mode')) {
      out.opencode_permission_mode = normalizeValue(
        values.opencode_permission_mode,
        OPENCODE_PERMISSION_VALUES,
        'default',
      );
    }
  }
  return out;
}

export function isApprovalSettingsMemoryEnabled(defaults: SpawnDefaults): boolean {
  // The preference is default-on for first use and for existing installations.
  // Only an explicit string "false" opts out.
  return defaults[REMEMBER_APPROVAL_SETTINGS_KEY] !== 'false';
}

export function defaultApprovalSettings(provider: string): SpawnApprovalSettings {
  if (Object.prototype.hasOwnProperty.call(PERMISSION_MODE_VALUES, provider)) {
    return { permission_mode: DEFAULT_APPROVAL_FORM_SETTINGS.permission_mode };
  }
  if (provider === 'codex') {
    return {
      sandbox: DEFAULT_APPROVAL_FORM_SETTINGS.sandbox,
      ask_for_approval: DEFAULT_APPROVAL_FORM_SETTINGS.ask_for_approval,
    };
  }
  if (provider === 'opencode') {
    return { opencode_permission_mode: DEFAULT_APPROVAL_FORM_SETTINGS.opencode_permission_mode };
  }
  return {};
}

/**
 * provider 別の名前空間キー（例 `permission_mode_claude`）から、
 * `selectedApprovalSettings` がそのまま読めるフラット名の値へ組み直す。
 * 名前空間キーが無ければ旧フラットキー（例 `permission_mode`）へフォールバックする。
 * 優先順位: 名前空間キー → 旧フラットキー → （どちらも無ければキー自体を作らず、
 * 呼び出し側の provider 既定に委ねる）。
 */
function namespacedApprovalValues(provider: string, defaults: SpawnDefaults): SpawnDefaults {
  const names = approvalKeyNamesForProvider(provider);
  const out: Record<string, unknown> = {};
  for (const name of names) {
    const nsKey = providerKey(name, provider);
    if (Object.prototype.hasOwnProperty.call(defaults, nsKey)) {
      out[name] = defaults[nsKey];
    } else if (Object.prototype.hasOwnProperty.call(defaults, name)) {
      out[name] = defaults[name];
    }
  }
  return out;
}

export function restoreApprovalSettings(provider: string, defaults: SpawnDefaults): SpawnApprovalSettings {
  if (!isApprovalSettingsMemoryEnabled(defaults)) return defaultApprovalSettings(provider);
  const namespaced = namespacedApprovalValues(provider, defaults);
  return { ...defaultApprovalSettings(provider), ...selectedApprovalSettings(provider, namespaced) };
}

/**
 * C3 (plan_spawn-form-per-provider-memory.md): 記憶 OFF (`remember=false`) の消去範囲。
 * `mergeApprovalSettings` は「記憶 OFF を書き込む」唯一の経路なので、許可設定だけでなく
 * subscription / worktree / 委譲の記憶も全 provider ぶんここで一緒に消す（部分的に OFF に
 * した provider だけ消して他の provider の記憶が残る、という状態を作らない）。
 * provider 名は `-` を含みうる（`cursor-agent` 等）ため、`<名前>_` の前方一致で
 * 「その名前の namespaced キー全部」を機械的に拾う。
 */
function isProviderScopedMemoryKey(key: string): boolean {
  const names: readonly string[] = [...APPROVAL_SETTING_KEYS, ...PROVIDER_BOOLEAN_SETTING_NAMES, 'subscription'];
  return names.some((name) => key === name || key.startsWith(`${name}_`));
}

export function mergeApprovalSettings(
  defaults: SpawnDefaults,
  provider: string,
  selected: SpawnDefaults,
  remember: boolean,
): Record<string, string> {
  const currentProviderKeys = new Set(
    approvalKeyNamesForProvider(provider).map((name) => providerKey(name, provider)),
  );
  const next: Record<string, string> = {};
  for (const [key, value] of Object.entries(defaults)) {
    if (key === REMEMBER_APPROVAL_SETTINGS_KEY || key === 'risk_confirmed') continue;
    // 旧フラットキーは provider を跨いで共有されていた残骸なので、常に捨てて移行する。
    if ((APPROVAL_SETTING_KEYS as readonly string[]).includes(key)) continue;
    // 現在の provider ぶんはこの後書き直すので、古い値をここで通さない。
    if (currentProviderKeys.has(key)) continue;
    // C3: 記憶 OFF のときは、他 provider ぶんも含めて記憶を丸ごと消す。
    // ON のとき（remember=true）は現在の provider の許可設定だけを書き換え、
    // 他 provider の記憶・他ドメイン（worktree/委譲/subscription）には触らない（従来どおり）。
    if (!remember && isProviderScopedMemoryKey(key)) continue;
    // 他 provider の名前空間キーはそのまま通す（干渉させない）。
    if (typeof value === 'string') next[key] = value;
  }
  next[REMEMBER_APPROVAL_SETTINGS_KEY] = remember ? 'true' : 'false';
  if (remember) {
    const chosen = selectedApprovalSettings(provider, selected);
    for (const [name, value] of Object.entries(chosen)) {
      next[providerKey(name, provider)] = value;
    }
  }
  return next;
}

/**
 * C2 (plan_spawn-form-per-provider-memory.md): worktree 隔離・委譲チェックボックスも
 * 許可設定と同じ provider 別名前空間（`providerKey` による `<名前>_<provider>`）で記憶する。
 * `spawn.defaults` は map[string]string なので、真偽値は 'true' / 'false' の文字列で往復する。
 *
 * この関数群を spawn-panel.ts の中に置かず、DOM 非依存のこのファイルへ置く理由:
 * spawn-panel.ts は session-list.ts 経由で app.ts を import しており、app.ts はモジュール
 * 直下（トップレベル）で `document.getElementById(...)` を呼ぶ（ガードなし）。import した
 * 時点で必ず評価されるため、spawn-panel.ts から何かを import するだけで DOM の無い
 * テスト実行環境（`bun test`）では即座に例外になる。spawn-confirm-store.ts が同じ理由で
 * DOM 非依存モジュールとして独立している（同ファイルの先頭コメント参照）のと同じ判断。
 */
export const PROVIDER_BOOLEAN_SETTING_NAMES = ['isolate_worktree', 'delegation'] as const;
export type ProviderBooleanSettingName = (typeof PROVIDER_BOOLEAN_SETTING_NAMES)[number];

/**
 * その provider の名前空間キー、または旧フラットキーのどちらかに値が記憶されているか。
 * C3: 記憶 OFF のときは、過去に書かれた値が残っていても「記憶あり」として扱わない
 * （`restoreApprovalSettings` が `isApprovalSettingsMemoryEnabled` を見るのと同じ扱い）。
 */
export function hasProviderBooleanSetting(
  name: ProviderBooleanSettingName,
  provider: string,
  defaults: SpawnDefaults,
): boolean {
  if (!isApprovalSettingsMemoryEnabled(defaults)) return false;
  const nsKey = providerKey(name, provider);
  return Object.prototype.hasOwnProperty.call(defaults, nsKey)
    || Object.prototype.hasOwnProperty.call(defaults, name);
}

/**
 * provider 付きキー → 旧フラットキー、の順に読む（優先順位は許可設定と同じ）。
 * どちらも無ければ false（未チェック）を返す。C3: 記憶 OFF のときは常に false
 * （provider を切り替えても各欄がその provider の既定で出るようにするため）。
 */
export function restoreProviderBooleanSetting(
  name: ProviderBooleanSettingName,
  provider: string,
  defaults: SpawnDefaults,
): boolean {
  if (!isApprovalSettingsMemoryEnabled(defaults)) return false;
  const nsKey = providerKey(name, provider);
  const raw = Object.prototype.hasOwnProperty.call(defaults, nsKey) ? defaults[nsKey] : defaults[name];
  return raw === 'true';
}

/**
 * 現在の provider ぶんの worktree / 委譲だけを書き換え、他 provider の名前空間キーは
 * そのまま通す。旧フラットキー（`isolate_worktree` / `delegation`）は常に捨てて移行する
 * （mergeApprovalSettings が許可設定の旧フラットキーを常に捨てるのと同じ順序）。
 */
export function mergeProviderBooleanSetting(
  defaults: SpawnDefaults,
  provider: string,
  values: Partial<Record<ProviderBooleanSettingName, boolean>>,
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const [key, value] of Object.entries(defaults)) {
    if ((PROVIDER_BOOLEAN_SETTING_NAMES as readonly string[]).includes(key)) continue;
    if (typeof value === 'string') next[key] = value;
  }
  for (const name of PROVIDER_BOOLEAN_SETTING_NAMES) {
    if (!Object.prototype.hasOwnProperty.call(values, name)) continue;
    next[providerKey(name, provider)] = values[name] ? 'true' : 'false';
  }
  return next;
}

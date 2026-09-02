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

function selectedApprovalSettings(provider: string, values: SpawnDefaults): SpawnApprovalSettings {
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

export function restoreApprovalSettings(provider: string, defaults: SpawnDefaults): SpawnApprovalSettings {
  if (!isApprovalSettingsMemoryEnabled(defaults)) return defaultApprovalSettings(provider);
  return { ...defaultApprovalSettings(provider), ...selectedApprovalSettings(provider, defaults) };
}

export function mergeApprovalSettings(
  defaults: SpawnDefaults,
  provider: string,
  selected: SpawnDefaults,
  remember: boolean,
): Record<string, string> {
  const next: Record<string, string> = {};
  for (const [key, value] of Object.entries(defaults)) {
    if (key === REMEMBER_APPROVAL_SETTINGS_KEY || key === 'risk_confirmed'
      || (APPROVAL_SETTING_KEYS as readonly string[]).includes(key)) continue;
    if (typeof value === 'string') next[key] = value;
  }
  next[REMEMBER_APPROVAL_SETTINGS_KEY] = remember ? 'true' : 'false';
  if (remember) Object.assign(next, selectedApprovalSettings(provider, selected));
  return next;
}

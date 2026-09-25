const RESERVED_PROVIDER_IDS = new Set([
  'claude',
  'codex',
  'copilot',
  'cursor-agent',
  'opencode',
  'grok',
  'command-code',
  'shell',
]);

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

export type ProviderOrigin =
  | 'embedded'
  | 'distribution'
  | 'user'
  | 'legacy'
  | 'override'
  | string;

export type ProviderStatusKind = 'available' | 'disabled' | 'missing' | 'error';

export type ProviderDiagnosticLike = {
  severity?: string;
  message?: string;
};

export function suggestProviderId(displayName: string): string {
  const slug = displayName
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  if (!slug) return '';
  let candidate = /^[a-z0-9]/.test(slug) ? slug : `p-${slug}`.slice(0, 64);
  if (!RESERVED_PROVIDER_IDS.has(candidate)) return candidate;
  candidate = `${candidate}-cli`.slice(0, 64);
  if (!RESERVED_PROVIDER_IDS.has(candidate)) return candidate;
  return `${candidate}-2`.slice(0, 64);
}

// duplicateProviderDraft picks the id for a "Duplicate" action: `<id>-copy`,
// then `-copy-2`, `-copy-3`, ... until it finds one not already in
// `existingIds`. The caller appends the i18n copy suffix to the display name
// itself; this only returns the plain "<display name or id>" text.
export function duplicateProviderDraft(
  source: { id: string; display_name?: string },
  existingIds: Set<string>,
): { id: string; displayName: string } {
  let candidate = `${source.id}-copy`;
  let suffix = 2;
  while (existingIds.has(candidate)) {
    candidate = `${source.id}-copy-${suffix}`;
    suffix += 1;
  }
  return { id: candidate, displayName: source.display_name || source.id };
}

export function originLabelKey(origin: ProviderOrigin): string {
  switch (origin) {
    case 'embedded':
      return 'settings_ai_providers_origin_embedded';
    case 'distribution':
      return 'settings_ai_providers_origin_distribution';
    case 'user':
      return 'settings_ai_providers_origin_user';
    case 'legacy':
      return 'settings_ai_providers_origin_legacy';
    case 'override':
      return 'settings_ai_providers_origin_override';
    default:
      return 'settings_ai_providers_origin_unknown';
  }
}

// providerStatusKind's priority is error > disabled > missing > available: a
// broken definition is worth flagging regardless of enabled state, and a
// missing executable stops mattering once the row is already turned off.
export function providerStatusKind(provider: { enabled: boolean; hasError?: boolean; commandMissing?: boolean }): ProviderStatusKind {
  if (provider.hasError) return 'error';
  if (!provider.enabled) return 'disabled';
  if (provider.commandMissing) return 'missing';
  return 'available';
}

export function statusLabelKey(kind: ProviderStatusKind): string {
  switch (kind) {
    case 'disabled':
      return 'settings_ai_providers_status_disabled';
    case 'missing':
      return 'settings_ai_providers_status_missing';
    case 'error':
      return 'settings_ai_providers_status_error';
    default:
      return 'settings_ai_providers_status_available';
  }
}

// RESERVED_PROVIDER_IDS minus the synthetic "shell" launch identity is the
// exact set the Hub API treats as history/override-backed (provider.IsBuiltinID
// on the Go side). Origin alone cannot tell this apart: once a built-in has
// been overridden once, its origin flips from "embedded" to "override", but
// it is still routed through the same per-provider revision contract as
// before the first edit.
export function isBuiltinProviderID(id: string): boolean {
  return id !== 'shell' && RESERVED_PROVIDER_IDS.has(id);
}

export type ProviderRequestFailureKind = 'network' | 'aborted' | 'http';

export type ProviderRequestFailureLike = {
  kind: ProviderRequestFailureKind;
  status?: number;
  code?: string;
  detail?: string;
  fields?: string[];
};

// providerErrorMessage is the single place that turns a failed provider API
// call into user-facing copy, so "offline", "409 conflict", and "5xx" each
// get their own message instead of one generic "request failed" for all
// three (the review-flagged gap: async failures used to have nowhere to
// surface distinctly).
export function providerErrorMessage(failure: ProviderRequestFailureLike): { key: string; fallback: string; vars: Record<string, unknown> } {
  if (failure.kind === 'network') {
    return { key: 'settings_ai_providers_offline', fallback: 'You appear to be offline. Check your connection and try again.', vars: {} };
  }
  // The Hub refused a built-in edit that would empty a field the distributed
  // definition fills in; only the Hub knows those values, so it names the
  // fields and this just says which ones to put back.
  if (failure.kind === 'http' && failure.code === 'provider_override_clears_value' && Array.isArray(failure.fields) && failure.fields.length > 0) {
    return {
      key: 'settings_ai_providers_cannot_clear',
      fallback: "These fields have a distributed default and can't be saved empty: {fields}. Keep the value or enter a different one.",
      vars: { fields: failure.fields.join(', ') },
    };
  }
  if (failure.kind === 'http' && failure.status === 409) {
    return { key: 'settings_ai_providers_revision_conflict', fallback: 'This was changed elsewhere. Reload and try again.', vars: {} };
  }
  if (failure.kind === 'http' && (failure.status ?? 0) >= 500) {
    return { key: 'settings_ai_providers_server_error', fallback: 'Server error ({status}). Try again shortly.', vars: { status: failure.status ?? 0 } };
  }
  // Any other 4xx that carries the Hub's reason shows it as sent: the
  // validation messages exist only on the Hub, so there is no table here to
  // translate them from (a 422 used to say only "Request failed (422).").
  if (failure.kind === 'http' && typeof failure.detail === 'string' && failure.detail !== '') {
    return {
      key: 'settings_ai_providers_request_failed_detail',
      fallback: 'Request failed ({status}): {detail}',
      vars: { status: failure.status ?? 0, detail: failure.detail },
    };
  }
  if (failure.kind === 'http') {
    return { key: 'settings_ai_providers_request_failed', fallback: 'Request failed ({status}).', vars: { status: failure.status ?? 0 } };
  }
  return { key: 'settings_ai_providers_request_failed_generic', fallback: 'Request failed.', vars: {} };
}

// createRequestGuard pins the fix for the review-flagged race: opening the
// edit form for provider A, then quickly switching to provider B before A's
// detail fetch resolves, used to let A's late response overwrite B's form.
// Each async lookup takes a token from begin() before it awaits, and only
// applies its result if isCurrent(token) is still true when it resolves.
export function createRequestGuard(): { begin: () => number; isCurrent: (token: number) => boolean } {
  let generation = 0;
  return {
    begin: () => {
      generation += 1;
      return generation;
    },
    isCurrent: (token: number) => token === generation,
  };
}

export function formatDiagnosticMessages(diagnostics: ProviderDiagnosticLike[]): { text: string; hasError: boolean } {
  const messages = diagnostics
    .map((diagnostic) => String(diagnostic?.message || '').trim())
    .filter(Boolean);
  return {
    text: messages.join(' '),
    hasError: diagnostics.some((diagnostic) => diagnostic?.severity === 'error'),
  };
}

export function visibleFocusableElements(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter((element) => {
    if (element.hidden || element.getAttribute('aria-hidden') === 'true') return false;
    return element.getClientRects().length > 0;
  });
}

export function cycleDialogFocus(root: HTMLElement, event: KeyboardEvent): boolean {
  if (event.key !== 'Tab') return false;
  const nodes = visibleFocusableElements(root);
  if (nodes.length === 0) return false;
  const first = nodes[0];
  const last = nodes[nodes.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
    return true;
  }
  if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
    return true;
  }
  return false;
}

// MAX_UPDATE_TIMEOUT_SECONDS mirrors maxUpdateTimeoutSec in
// internal/provider/update.go; DEFAULT_UPDATE_TIMEOUT_SECONDS mirrors
// defaultUpdateTimeoutSec there. Both are display-only here (an empty form
// field means "use the server's default", never a literal 0 sent to save).
export const MAX_UPDATE_TIMEOUT_SECONDS = 3600;
export const DEFAULT_UPDATE_TIMEOUT_SECONDS = 300;

export type ProviderUpdateFormValues = {
  enabled: boolean;
  versionArgs: string;
  executable: string;
  args: string;
  timeoutSeconds: string;
};

function stringArrayField(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

// splitArgLine turns the form's one-line, space-separated input back into an
// argv array. Quoted/whitespace-containing arguments are out of scope on
// purpose (the form's hint text says to use the advanced JSON editor for
// those) so this never has to reimplement shell-style quote parsing.
function splitArgLine(value: string): string[] {
  return value.trim().split(/\s+/).filter(Boolean);
}

// providerUpdateFormValuesFromUpdate reads a provider's `update` block (or
// `undefined`/`null` for a definition with no block at all, e.g. a freshly
// added custom provider) into the flat strings the settings form's inputs
// use. `enabled` falls back to "args is non-empty" when the block omits an
// explicit flag, mirroring UpdateEnabled in internal/provider/update.go so
// the form's initial toggle state agrees with what the server would compute.
export function providerUpdateFormValuesFromUpdate(update: unknown): ProviderUpdateFormValues {
  const source = (update && typeof update === 'object' && !Array.isArray(update)) ? update as Record<string, unknown> : {};
  const versionArgs = stringArrayField(source.version_args);
  const args = stringArrayField(source.args);
  const enabled = typeof source.enabled === 'boolean' ? source.enabled : args.length > 0;
  const timeoutSeconds = typeof source.timeout_seconds === 'number' && source.timeout_seconds > 0
    ? String(source.timeout_seconds)
    : '';
  return {
    enabled,
    versionArgs: versionArgs.join(' '),
    executable: typeof source.executable === 'string' ? source.executable : '',
    args: args.join(' '),
    timeoutSeconds,
  };
}

export type ProviderUpdateFormError = { key: string; fallback: string };

export type ProviderUpdateFromFormResult =
  | { ok: true; update: Record<string, unknown> }
  | { ok: false; error: ProviderUpdateFormError };

// providerUpdateFromFormValues is the inverse of
// providerUpdateFormValuesFromUpdate. It always returns every update.* key,
// using an explicit `undefined` (never an omitted key) for anything the form
// leaves blank. That matters to the caller in provider-manager.ts: the
// result is spread over whatever `update` block the advanced-JSON textarea
// still carries, and only an explicit `undefined` in the spread's later
// object wins over — and then JSON.stringify drops — a stale key from
// there. An omitted key would leave the old value in place instead of
// clearing it.
export function providerUpdateFromFormValues(values: ProviderUpdateFormValues): ProviderUpdateFromFormResult {
  const versionArgs = splitArgLine(values.versionArgs);
  const args = splitArgLine(values.args);
  const executable = values.executable.trim();
  if (values.enabled && args.length === 0) {
    return {
      ok: false,
      error: { key: 'settings_ai_providers_update_enabled_without_args', fallback: 'Enter an update command before turning this on.' },
    };
  }
  let timeoutSeconds: number | undefined;
  const timeoutRaw = values.timeoutSeconds.trim();
  if (timeoutRaw !== '') {
    const parsed = Number(timeoutRaw);
    if (!Number.isInteger(parsed) || parsed < 0 || parsed > MAX_UPDATE_TIMEOUT_SECONDS) {
      return {
        ok: false,
        error: {
          key: 'settings_ai_providers_update_timeout_invalid',
          fallback: `Timeout must be a whole number of seconds between 0 and ${MAX_UPDATE_TIMEOUT_SECONDS}.`,
        },
      };
    }
    timeoutSeconds = parsed;
  }
  return {
    ok: true,
    update: {
      version_args: versionArgs.length > 0 ? versionArgs : undefined,
      args: args.length > 0 ? args : undefined,
      executable: executable || undefined,
      enabled: values.enabled,
      timeout_seconds: timeoutSeconds,
    },
  };
}

// providerUpdateIsDefault reports whether a provider's `update` block still
// matches its distributed/embedded value. field_origins is stamped
// uniformly across every top-level key of a provider the instant any layer
// overrides that provider at all (registry.go mergeLayers rebuilds the whole
// fields map from the last layer's source on every merge, not just the keys
// that layer actually set), so this cannot isolate "only update changed"
// from "some other field changed". It answers the coarser question the
// settings form actually needs instead: "has this provider ever been
// edited", which is what determines whether the existing whole-provider
// "reset to distributed default" action would revert this section too.
export function providerUpdateIsDefault(fieldOrigins: unknown): boolean {
  if (!fieldOrigins || typeof fieldOrigins !== 'object') return true;
  const entry = (fieldOrigins as Record<string, unknown>).update;
  if (!entry || typeof entry !== 'object') return true;
  const origin = (entry as Record<string, unknown>).origin;
  return origin === 'embedded' || origin === undefined || origin === '';
}

export function providerUpdateLoginMayBeRequired(update: unknown): boolean {
  return !!(update && typeof update === 'object' && (update as Record<string, unknown>).login_may_be_required === true);
}

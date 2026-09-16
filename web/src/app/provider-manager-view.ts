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

export type ProviderRequestFailureLike = { kind: ProviderRequestFailureKind; status?: number };

// providerErrorMessage is the single place that turns a failed provider API
// call into user-facing copy, so "offline", "409 conflict", and "5xx" each
// get their own message instead of one generic "request failed" for all
// three (the review-flagged gap: async failures used to have nowhere to
// surface distinctly).
export function providerErrorMessage(failure: ProviderRequestFailureLike): { key: string; fallback: string; vars: Record<string, unknown> } {
  if (failure.kind === 'network') {
    return { key: 'settings_ai_providers_offline', fallback: 'You appear to be offline. Check your connection and try again.', vars: {} };
  }
  if (failure.kind === 'http' && failure.status === 409) {
    return { key: 'settings_ai_providers_conflict', fallback: 'This was changed elsewhere. Reload and try again.', vars: {} };
  }
  if (failure.kind === 'http' && (failure.status ?? 0) >= 500) {
    return { key: 'settings_ai_providers_server_error', fallback: 'Server error ({status}). Try again shortly.', vars: { status: failure.status ?? 0 } };
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

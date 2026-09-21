import { apiFetch } from './util.js';

export {
  BUILTIN_PROVIDER_CAPABILITIES,
  DEFAULT_PROVIDER_CAPABILITIES,
  cacheProviderCapabilities,
  clearProviderCapabilitiesCache,
  hasProviderCapability,
  providerCapabilitiesFor,
  setProviderCapability,
  type ProviderCapabilities,
} from './provider-capabilities.js';
import {
  cacheProviderCapabilities,
  setProviderCapability,
  type ProviderCapabilities,
} from './provider-capabilities.js';

export type ProviderSummary = {
  id: string;
  display_name: string;
  enabled: boolean;
  origin: string;
  revision: string;
  capabilities: ProviderCapabilities;
};

export type ProviderDiagnostic = {
  code?: string;
  severity?: string;
  field?: string;
  message?: string;
};

export type ProviderListResponse = {
  revision: string;
  providers: ProviderSummary[];
  diagnostics: ProviderDiagnostic[];
};

export type ProviderSourceRef = {
  origin?: string;
  version?: string;
  digest?: string;
  revision?: string;
};

// A loosely-typed mirror of internal/provider.Definition. Fields the UI does
// not render are still round-tripped (kept as unknown) so an edit through the
// advanced JSON editor cannot silently drop launch/model/adapters data the
// basic form fields never touch.
export type ProviderDefinition = {
  schema_version?: number;
  id?: string;
  display_name?: string;
  description?: string;
  enabled?: boolean;
  launch?: Record<string, unknown>;
  models?: Record<string, unknown>;
  capabilities?: Record<string, boolean>;
  adapters?: Record<string, unknown>;
  presentation?: Record<string, unknown>;
  approval_pattern_source?: string;
  [key: string]: unknown;
};

export type ProviderEffectiveDefinition = ProviderDefinition & {
  effective_source: ProviderSourceRef;
  revision: string;
  capabilities_summary?: ProviderCapabilities;
};

export type ProviderRequestFailure =
  | { ok: false; kind: 'network' }
  | { ok: false; kind: 'aborted' }
  | { ok: false; kind: 'http'; status: number; code?: string; detail?: string };

export type ProviderDetailResult =
  | { ok: true; revision: string; provider: ProviderEffectiveDefinition; diagnostics: ProviderDiagnostic[] }
  | ProviderRequestFailure;

export type ProviderMutationResult =
  | { ok: true; revision: string; diagnostics: ProviderDiagnostic[] }
  | ProviderRequestFailure;

async function providerFetchJSON(
  url: string,
  init: RequestInit | undefined,
  signal: AbortSignal | undefined,
): Promise<
  | { outcome: 'ok'; status: number; body: any }
  | { outcome: 'http-error'; status: number; body: any }
  | { outcome: 'network' }
  | { outcome: 'aborted' }
> {
  try {
    const response = await apiFetch(url, signal ? { ...init, signal } : init);
    let body: any = null;
    try {
      body = await response.json();
    } catch (_) {
      body = null;
    }
    return response.ok
      ? { outcome: 'ok', status: response.status, body }
      : { outcome: 'http-error', status: response.status, body };
  } catch (error) {
    if (error instanceof Error && error.name === 'AbortError') return { outcome: 'aborted' };
    return { outcome: 'network' };
  }
}

function providerHTTPFailure(status: number, body: any): ProviderRequestFailure {
  return {
    ok: false,
    kind: 'http',
    status,
    code: typeof body?.error === 'string' ? body.error : undefined,
    detail: typeof body?.detail === 'string' ? body.detail : undefined,
  };
}

export async function loadProviderSummaries(options?: { includeDisabled?: boolean }): Promise<ProviderListResponse | null> {
  try {
    const response = await apiFetch(`/api/providers`, {
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) return null;
    const body = await response.json();
    if (!body || !Array.isArray(body.providers)) return null;
    cacheProviderCapabilities(body.providers);
    const includeDisabled = options?.includeDisabled === true;
    return {
      revision: typeof body.revision === 'string' ? body.revision : '',
      providers: body.providers.filter((entry: ProviderSummary) => (
        entry && typeof entry.id === 'string' && (includeDisabled || entry.enabled !== false)
      )),
      diagnostics: Array.isArray(body.diagnostics) ? body.diagnostics : [],
    };
  } catch (_) {
    return null;
  }
}

export async function validateProviderDefinition(definition: unknown): Promise<{ valid: boolean; diagnostics: ProviderDiagnostic[] } | null> {
  try {
    const response = await apiFetch(`/api/providers/validate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify(definition),
    });
    if (!response.ok) return null;
    const body = await response.json();
    return {
      valid: body?.valid === true,
      diagnostics: Array.isArray(body?.diagnostics) ? body.diagnostics : [],
    };
  } catch (_) {
    return null;
  }
}

// loadProviderDetail, unlike loadProviderSummaries/validateProviderDefinition
// above, distinguishes offline / HTTP-error / aborted instead of collapsing
// every failure into `null`, so callers can show the right message (and
// silently ignore an aborted request instead of treating it as a failure).
export async function loadProviderDetail(id: string, options?: { signal?: AbortSignal }): Promise<ProviderDetailResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  const body = result.body;
  if (!body || typeof body.provider !== 'object' || body.provider === null) {
    return { ok: false, kind: 'network' };
  }
  const eff = body.provider as ProviderEffectiveDefinition;
  if (eff.capabilities_summary) {
    setProviderCapability(id, eff.capabilities_summary);
  }
  return {
    ok: true,
    revision: typeof body.revision === 'string' ? body.revision : '',
    provider: eff,
    diagnostics: Array.isArray(body.diagnostics) ? body.diagnostics : [],
  };
}

export async function createProvider(definition: unknown, options?: { signal?: AbortSignal }): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers`,
    { method: 'POST', headers: { 'Content-Type': 'application/json', Accept: 'application/json' }, body: JSON.stringify(definition) },
    options?.signal,
  );
  return providerMutationOutcome(result);
}

// saveProviderDefinition is shared by the built-in override path and the
// custom-provider edit path: both are a PATCH against the same endpoint, and
// the Hub dispatches on provider.IsBuiltinID internally. expectedRevision is
// the per-provider HistoryStore revision for a built-in id (empty string
// until it has ever been overridden), or the whole-registry revision for a
// custom id — callers must fetch the correct one from loadProviderDetail /
// loadProviderSummaries first, never invent it.
export async function saveProviderDefinition(
  id: string,
  expectedRevision: string,
  definition: unknown,
  options?: { signal?: AbortSignal },
): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}`,
    {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ expected_revision: expectedRevision, definition }),
    },
    options?.signal,
  );
  return providerMutationOutcome(result);
}

// removeProvider is DELETE for both a built-in (soft-disables via an
// override) and a custom provider (actually removes the file) — same request
// shape either way; the Hub decides which by provider.IsBuiltinID.
export async function removeProvider(id: string, expectedRevision: string, options?: { signal?: AbortSignal }): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}?expected_revision=${encodeURIComponent(expectedRevision)}`,
    { method: 'DELETE', headers: { Accept: 'application/json' } },
    options?.signal,
  );
  return providerMutationOutcome(result);
}

export async function restoreProviderRevision(
  id: string,
  revision: string,
  expectedRevision: string,
  options?: { signal?: AbortSignal },
): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/restore`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ revision, expected_revision: expectedRevision }),
    },
    options?.signal,
  );
  return providerMutationOutcome(result);
}

function providerMutationOutcome(result: Awaited<ReturnType<typeof providerFetchJSON>>): ProviderMutationResult {
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return {
    ok: true,
    revision: typeof result.body?.revision === 'string' ? result.body.revision : '',
    diagnostics: Array.isArray(result.body?.diagnostics) ? result.body.diagnostics : [],
  };
}

export type ProviderRevisionRecord = {
  schema_version: number;
  provider_id: string;
  revision: string;
  created_at: string;
  reason: string;
  parent_revision?: string;
  content_digest: string;
  payload: ProviderDefinition;
};

export type ProviderHistoryResult =
  | { ok: true; revisions: ProviderRevisionRecord[] }
  | ProviderRequestFailure;

export async function loadProviderHistory(id: string, options?: { signal?: AbortSignal }): Promise<ProviderHistoryResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/history`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return { ok: true, revisions: Array.isArray(result.body?.revisions) ? result.body.revisions : [] };
}

export type ProviderHistoryDiffResult =
  | { ok: true; diff: unknown[]; currentRevision: string }
  | ProviderRequestFailure;

export async function loadProviderRevisionDiff(id: string, revision: string, options?: { signal?: AbortSignal }): Promise<ProviderHistoryDiffResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/history/${encodeURIComponent(revision)}/diff`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return {
    ok: true,
    diff: Array.isArray(result.body?.diff) ? result.body.diff : [],
    currentRevision: typeof result.body?.current_revision === 'string' ? result.body.current_revision : '',
  };
}

export async function resetProviderOverride(id: string, expectedRevision: string): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/reset`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ expected_revision: expectedRevision }),
    },
    undefined,
  );
  return providerMutationOutcome(result);
}

export type ProviderBackupsResult =
  | { ok: true; backups: ProviderRevisionRecord[] }
  | ProviderRequestFailure;

// loadProviderHistory/loadProviderBackups are the UI's counterpart to the
// `many-ai-cli provider backup list` CLI command — both call the same
// HistoryStore methods on the Hub side (see provider_handlers.go), so a
// backup either tool can see is the same backup the other can restore.
export async function loadProviderBackups(id: string, options?: { signal?: AbortSignal }): Promise<ProviderBackupsResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/backups`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return { ok: true, backups: Array.isArray(result.body?.backups) ? result.body.backups : [] };
}

export type ProviderBackupVerifyResult =
  | { ok: true; backup: ProviderRevisionRecord }
  | ProviderRequestFailure;

export async function verifyProviderBackup(id: string, backupId: string, options?: { signal?: AbortSignal }): Promise<ProviderBackupVerifyResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/backups/${encodeURIComponent(backupId)}/verify`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return { ok: true, backup: result.body?.backup as ProviderRevisionRecord };
}

export async function restoreProviderBackup(
  id: string,
  backupId: string,
  expectedRevision: string,
  options?: { signal?: AbortSignal },
): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/backups/${encodeURIComponent(backupId)}/restore`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ expected_revision: expectedRevision }),
    },
    options?.signal,
  );
  return providerMutationOutcome(result);
}

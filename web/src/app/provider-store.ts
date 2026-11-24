import { apiFetch } from './util.js';
import { applyProviderPresentation } from './provider-icon.js';
import { cacheProviderDeclarations, hasProviderDeclarations } from './provider-declarations.js';
import { seedBuiltinProviderDeclarations } from './provider-declarations-seed.js';

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
  // 利用者が設定した AI アイコンの頭文字と色。Hub が検査に通ったものだけを載せる（無ければ省略）。
  presentation?: { icon_text?: string; color?: string };
  // 利用者が AI のアイコンに画像を選んでいるときだけ付く短い版。画像 URL の ?v= に使う（差し替え直後の古い画像を避ける）。
  icon_image_version?: string;
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
  update?: Record<string, unknown>;
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
  | { ok: false; kind: 'http'; status: number; code?: string; detail?: string; fields?: string[] };

export type ProviderDetailResult =
  | { ok: true; revision: string; provider: ProviderEffectiveDefinition; diagnostics: ProviderDiagnostic[] }
  | ProviderRequestFailure;

export type ProviderMutationResult =
  | { ok: true; revision: string; diagnostics: ProviderDiagnostic[] }
  | ProviderRequestFailure;

type ProviderFetchOutcome =
  | { outcome: 'ok'; status: number; body: any }
  | { outcome: 'http-error'; status: number; body: any }
  | { outcome: 'network' }
  | { outcome: 'aborted' };

// 同梱の AI CLI の定義を変える要求（編集・無効化・初期化・復元・復旧）は、token に加えて画面の印
// （ページを読み込んだときに Hub が出す cookie）が要る。印はページを読み込んだときにだけ出し直され、
// 24 時間で切れる。開いたままの画面で断られたら（403 ui_required）、ページの入口を 1 回読んで印を
// 出し直し、同じ要求を 1 回だけ送り直す。それでも断られたら、呼び出し側が「ページを開き直して」を出す。
async function providerFetchJSON(
  url: string,
  init: RequestInit | undefined,
  signal: AbortSignal | undefined,
): Promise<ProviderFetchOutcome> {
  const first = await providerFetchOnce(url, init, signal);
  if (!refusedForDashboardProof(first, init)) return first;
  if (!(await renewDashboardProof(signal))) return first;
  return providerFetchOnce(url, init, signal);
}

function refusedForDashboardProof(result: ProviderFetchOutcome, init: RequestInit | undefined): boolean {
  const method = (init?.method || 'GET').toUpperCase();
  return method !== 'GET'
    && result.outcome === 'http-error'
    && result.status === 403
    && result.body?.error === 'ui_required';
}

// ページの入口（GET /）は token なしで読め、応答で画面の印を出し直す。中身は使わない。
async function renewDashboardProof(signal: AbortSignal | undefined): Promise<boolean> {
  try {
    const response = await fetch('/', {
      credentials: 'same-origin',
      cache: 'no-store',
      ...(signal ? { signal } : {}),
    });
    try { await response.arrayBuffer(); } catch (_) { /* 読み捨て */ }
    return response.ok;
  } catch (_) {
    return false;
  }
}

async function providerFetchOnce(
  url: string,
  init: RequestInit | undefined,
  signal: AbortSignal | undefined,
): Promise<ProviderFetchOutcome> {
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
    // Field names the Hub refused to save empty (422 provider_override_clears_value).
    fields: Array.isArray(body?.fields) && body.fields.every((field: unknown) => typeof field === 'string')
      ? body.fields
      : undefined,
  };
}

// 宣言（表示名・頭文字・並び順・使用量ページの URL・許可モードの選択肢・入力の癖）を Hub から取り直す。
// 一覧と同じ経路で、同梱 7 本も登録した AI CLI も同じ形で届く。取れなければ前の値のまま（画面は id から作る既定で動く）。
export async function loadProviderDeclarations(): Promise<boolean> {
  try {
    const response = await apiFetch(`/api/providers/declarations`, {
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) return false;
    const body = await response.json();
    if (!body || !Array.isArray(body.declarations)) return false;
    cacheProviderDeclarations(body.declarations);
    return true;
  } catch (_) {
    return false;
  }
}

// 起動時に 1 回。Hub の答えが届くまで（と、取れなかったとき）は同梱 7 本の種を置いておく。
// 種が無いと、取得前や失敗時に同梱の CLI が「登録した AI CLI」に見え、起動確認の画面が
// 許可モードなどの行を隠してしまう。Hub の答えが届けば丸ごと入れ替わる（一覧を取り直すたびに、もう一度取る）。
export function initProviderDeclarations(): void {
  if (!hasProviderDeclarations()) seedBuiltinProviderDeclarations();
  void loadProviderDeclarations();
}

export async function loadProviderSummaries(options?: { includeDisabled?: boolean }): Promise<ProviderListResponse | null> {
  // 一覧に使う名前・頭文字・並び順を先に揃える（一覧と同じ取り直しの口で）。
  await loadProviderDeclarations();
  try {
    const response = await apiFetch(`/api/providers`, {
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) return null;
    const body = await response.json();
    if (!body || !Array.isArray(body.providers)) return null;
    cacheProviderCapabilities(body.providers);
    // 一覧を取るたびに、利用者が設定したアイコンの頭文字と色を全画面へ反映する（起動時の取得・保存後の取り直しも同じ経路）。
    applyProviderPresentation(body.providers);
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

// withoutServerComputedFields drops what the Hub computes when it answers a
// read and never takes back on a save (source/effective_source/revision/
// field_origins/capabilities_summary). Every caller that sends a loaded
// definition back through PATCH uses it, so they agree on what "the whole
// definition" is.
export function withoutServerComputedFields(definition: ProviderEffectiveDefinition): Record<string, unknown> {
  const rest: Record<string, unknown> = { ...(definition as unknown as Record<string, unknown>) };
  delete rest.source;
  delete rest.effective_source;
  delete rest.revision;
  delete rest.field_origins;
  delete rest.capabilities_summary;
  return rest;
}

// setProviderUpdateEnabled flips only update.enabled — whether this AI CLI takes
// part in the updater ("全部更新" and its own 更新 button). It loads the current
// definition and sends the whole thing back, the same contract the Enable button
// and the edit dialog use.
export async function setProviderUpdateEnabled(id: string, enabled: boolean): Promise<ProviderMutationResult> {
  const detail = await loadProviderDetail(id);
  if (detail.ok === false) return detail;
  const definition = withoutServerComputedFields(detail.provider);
  const update = { ...((definition.update as Record<string, unknown> | undefined) || {}), enabled };
  return saveProviderDefinition(id, detail.provider.effective_source?.revision || '', { ...definition, update });
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

// AI のアイコンに使う画像。PUT は画像の中身をそのまま送る（形式は Hub が中身から判定する）。
// 送る前の確認（形式・512 KB）は provider-manager-view.ts の checkIconImageFile が受け持つ。
export async function uploadProviderIconImage(
  id: string,
  file: Blob,
  options?: { signal?: AbortSignal },
): Promise<{ ok: true } | ProviderRequestFailure> {
  let body: ArrayBuffer;
  try {
    body = await file.arrayBuffer();
  } catch (_) {
    return { ok: false, kind: 'network' };
  }
  const result = await providerFetchJSON(
    `/api/provider-icons/${encodeURIComponent(id)}`,
    { method: 'PUT', headers: { 'Content-Type': file.type || 'application/octet-stream', Accept: 'application/json' }, body },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return { ok: true };
}

export async function removeProviderIconImage(id: string, options?: { signal?: AbortSignal }): Promise<{ ok: true } | ProviderRequestFailure> {
  const result = await providerFetchJSON(
    `/api/provider-icons/${encodeURIComponent(id)}`,
    { method: 'DELETE', headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  return { ok: true };
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

export type ProviderRecoveryCandidate =
  | { kind: 'user_revision'; revision: string; created_at: string }
  | { kind: 'base' };

export type ProviderRecoveryResult =
  | { ok: true; provider_id: string; state: 'ok' | 'recovery_required'; candidates: ProviderRecoveryCandidate[] }
  | ProviderRequestFailure;

// loadProviderRecovery/recoverProvider back the history dialog's "recovery
// required" panel: when the provider's on-disk config could not be parsed at
// Hub startup, the history dialog needs to know so it can offer the last
// readable user revision (or the distributed base) as a restore target,
// instead of showing an empty/broken history list.
export async function loadProviderRecovery(id: string, options?: { signal?: AbortSignal }): Promise<ProviderRecoveryResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/recovery`,
    { headers: { Accept: 'application/json' } },
    options?.signal,
  );
  if (result.outcome === 'aborted') return { ok: false, kind: 'aborted' };
  if (result.outcome === 'network') return { ok: false, kind: 'network' };
  if (result.outcome === 'http-error') return providerHTTPFailure(result.status, result.body);
  const rawCandidates = Array.isArray(result.body?.candidates) ? result.body.candidates : [];
  const candidates: ProviderRecoveryCandidate[] = rawCandidates.map((candidate: any) =>
    candidate?.kind === 'user_revision'
      ? {
          kind: 'user_revision',
          revision: typeof candidate.revision === 'string' ? candidate.revision : '',
          created_at: typeof candidate.created_at === 'string' ? candidate.created_at : '',
        }
      : { kind: 'base' },
  );
  return {
    ok: true,
    provider_id: typeof result.body?.provider_id === 'string' ? result.body.provider_id : id,
    state: result.body?.state === 'recovery_required' ? 'recovery_required' : 'ok',
    candidates,
  };
}

export async function recoverProvider(id: string, revision: string): Promise<ProviderMutationResult> {
  const result = await providerFetchJSON(
    `/api/providers/${encodeURIComponent(id)}/recovery`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ revision }),
    },
    undefined,
  );
  return providerMutationOutcome(result);
}

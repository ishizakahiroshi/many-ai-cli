import { token } from './util.js';

export type ProviderCapabilities = {
  launch: boolean;
  models: boolean;
  effort: boolean;
  headless: boolean;
  approval: boolean;
  transcript: boolean;
  usage: boolean;
  subscription: boolean;
  permissions: boolean;
};

export type ProviderSummary = {
  id: string;
  display_name: string;
  enabled: boolean;
  origin: string;
  revision: string;
  capabilities: ProviderCapabilities;
};

export type ProviderListResponse = {
  revision: string;
  providers: ProviderSummary[];
};

export async function loadProviderSummaries(): Promise<ProviderListResponse | null> {
  try {
    const response = await fetch(`/api/providers?token=${encodeURIComponent(token || '')}`, {
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) return null;
    const body = await response.json();
    if (!body || !Array.isArray(body.providers)) return null;
    return {
      revision: typeof body.revision === 'string' ? body.revision : '',
      providers: body.providers.filter((entry: ProviderSummary) => (
        entry && typeof entry.id === 'string' && entry.enabled !== false
      )),
    };
  } catch (_) {
    return null;
  }
}

export async function validateProviderDefinition(definition: unknown): Promise<{ valid: boolean; diagnostics: unknown[] } | null> {
  try {
    const response = await fetch(`/api/providers/validate?token=${encodeURIComponent(token || '')}`, {
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

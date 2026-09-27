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

export const BUILTIN_PROVIDER_CAPABILITIES: Readonly<Record<string, ProviderCapabilities>> = Object.freeze({
  claude: Object.freeze({
    launch: true,
    models: true,
    effort: true,
    headless: true,
    approval: true,
    transcript: true,
    usage: true,
    subscription: true,
    permissions: true,
  }),
  codex: Object.freeze({
    launch: true,
    models: true,
    effort: true,
    headless: false,
    approval: true,
    transcript: true,
    usage: true,
    subscription: true,
    permissions: true,
  }),
  copilot: Object.freeze({
    launch: true,
    models: true,
    effort: false,
    headless: false,
    approval: true,
    transcript: false,
    usage: false,
    subscription: false,
    permissions: true,
  }),
  'cursor-agent': Object.freeze({
    launch: true,
    models: true,
    effort: false,
    headless: false,
    approval: true,
    transcript: false,
    usage: false,
    subscription: false,
    permissions: true,
  }),
  opencode: Object.freeze({
    launch: true,
    models: true,
    effort: false,
    headless: false,
    approval: true,
    transcript: false,
    usage: false,
    subscription: false,
    permissions: true,
  }),
  grok: Object.freeze({
    launch: true,
    models: true,
    effort: false,
    headless: false,
    approval: true,
    transcript: false,
    usage: true,
    subscription: true,
    permissions: true,
  }),
  'command-code': Object.freeze({
    launch: true,
    models: true,
    effort: false,
    headless: false,
    approval: true,
    transcript: true,
    usage: false,
    subscription: false,
    permissions: true,
  }),
  shell: Object.freeze({
    launch: true,
    models: false,
    effort: false,
    headless: false,
    approval: false,
    transcript: false,
    usage: false,
    subscription: false,
    permissions: false,
  }),
});

export const DEFAULT_PROVIDER_CAPABILITIES: Readonly<ProviderCapabilities> = Object.freeze({
  launch: true,
  models: false,
  effort: false,
  headless: false,
  approval: false,
  transcript: false,
  usage: false,
  subscription: false,
  permissions: false,
});

const providerCapabilitiesCache = new Map<string, ProviderCapabilities>();

export function clearProviderCapabilitiesCache(): void {
  providerCapabilitiesCache.clear();
}

export function cacheProviderCapabilities(providers: Iterable<{ id?: unknown; capabilities?: unknown }>): void {
  for (const p of providers) {
    if (p && typeof p.id === 'string' && p.capabilities && typeof p.capabilities === 'object') {
      providerCapabilitiesCache.set(p.id, p.capabilities as ProviderCapabilities);
    }
  }
}

export function setProviderCapability(id: string, capabilities: ProviderCapabilities): void {
  if (id) {
    providerCapabilitiesCache.set(id, capabilities);
  }
}

export function providerCapabilitiesFor(id: string): ProviderCapabilities {
  if (!id) return DEFAULT_PROVIDER_CAPABILITIES;
  const cached = providerCapabilitiesCache.get(id);
  if (cached) return cached;
  const builtin = BUILTIN_PROVIDER_CAPABILITIES[id];
  if (builtin) return builtin;
  return DEFAULT_PROVIDER_CAPABILITIES;
}

export function hasProviderCapability(id: string, capability: keyof ProviderCapabilities): boolean {
  return !!providerCapabilitiesFor(id)[capability];
}

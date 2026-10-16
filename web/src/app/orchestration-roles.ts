// Orchestration role choices shared by the spawn settings and relay dialog.

export interface OrchestrationCLIOption {
  value: string;
  label?: string;
  labelKey?: string;
}

export const ORCHESTRATION_ROLE_DEFS = [
  { key: 'implementation', labelKey: 'spawn_role_implementation' },
  { key: 'implementation-strong', labelKey: 'spawn_role_implementation_strong' },
  { key: 'test', labelKey: 'spawn_role_test' },
  { key: 'review', labelKey: 'spawn_role_review' },
] as const;

// Shell is not a useful child-role CLI choice, so it is intentionally omitted.
export const ORCHESTRATION_CLI_OPTIONS: readonly OrchestrationCLIOption[] = [
  { value: '',             labelKey: 'spawn_role_cli_none' },
  { value: 'claude',       label: 'Claude Code' },
  { value: 'codex',        label: 'Codex CLI' },
  { value: 'copilot',      label: 'GitHub Copilot' },
  { value: 'cursor-agent', label: 'Cursor Agent' },
  { value: 'opencode',     label: 'OpenCode' },
  { value: 'grok',         label: 'Grok Build' },
  { value: 'command-code', label: 'Command Code' },
] as const;

// /api/info covers config customs; /api/providers also covers Settings-added
// providers. Keep these sources separate so a later config-only response cannot
// remove live registry choices. Registry exclusions override the config fallback.
let customCLIOptions: OrchestrationCLIOption[] = [];
let registryCLIOptions: OrchestrationCLIOption[] = [];
let unavailableRegistryIDs = new Set<string>();
let registryProvidersLoaded = false;
const providerChoiceListeners = new Set<() => void>();

export function getOrchestrationCLIOptions(): readonly OrchestrationCLIOption[] {
  const choices = new Map<string, OrchestrationCLIOption>();
  for (const choice of [...ORCHESTRATION_CLI_OPTIONS, ...customCLIOptions, ...registryCLIOptions]) {
    if (!unavailableRegistryIDs.has(choice.value)) choices.set(choice.value, choice);
  }
  return [...choices.values()];
}

export function setOrchestrationCustomProviders(entries: unknown): void {
  if (!Array.isArray(entries)) return; // Older Hub, or a response without this field.
  const seen = new Set([...ORCHESTRATION_CLI_OPTIONS.map((choice) => choice.value), 'shell']);
  const next: OrchestrationCLIOption[] = [];
  for (const entry of entries) {
    const value = typeof entry?.id === 'string' ? entry.id.trim() : '';
    if (!value || seen.has(value)) continue;
    seen.add(value);
    const label = typeof entry?.label === 'string' && entry.label.trim() ? entry.label : value;
    next.push({ value, label });
  }
  customCLIOptions = next;
  for (const listener of providerChoiceListeners) listener();
}

// Feed the same generation-checked registry response used by ordinary spawn.
// A failed request leaves the last successful snapshot intact. An explicit empty
// list clears registry extras, while config fallback remains available.
export function setOrchestrationRegistryProviders(entries: unknown): void {
  if (!Array.isArray(entries)) return;
  const next: OrchestrationCLIOption[] = [];
  const unavailable = new Set<string>();
  const seen = new Set(['', 'shell']);
  for (const entry of entries) {
    const value = typeof entry?.id === 'string' ? entry.id.trim() : '';
    if (seen.has(value)) continue;
    seen.add(value);
    if (entry.enabled === false || entry.capabilities?.launch !== true) {
      unavailable.add(value);
      continue;
    }
    const label = typeof entry.display_name === 'string' && entry.display_name.trim() ? entry.display_name : value;
    next.push({ value, label });
  }
  registryCLIOptions = next;
  unavailableRegistryIDs = unavailable;
  registryProvidersLoaded = true;
  for (const listener of providerChoiceListeners) listener();
}

export function onOrchestrationProvidersChanged(listener: () => void): () => void {
  providerChoiceListeners.add(listener);
  return () => { providerChoiceListeners.delete(listener); };
}

// Use DOM properties, not HTML interpolation: custom labels and ids are data.
// Replacing only options leaves the select, its listeners and the other fields
// intact. A removed role choice falls back to unset. A pending confirmation
// retains its exact selection even when a discovery response does not list it:
// the Hub validates approval, and discovery must never silently change its CLI.
export function fillOrchestrationProviderSelect(
  select: HTMLSelectElement,
  translate: (key: string) => string,
  selected = select.value,
  includeUnset = true,
): void {
  const choices = getOrchestrationCLIOptions().filter((choice) => includeUnset || choice.value);
  // Saved roles can arrive before the registry response. Confirmation choices
  // are also kept after discovery, including across removals or failed refreshes.
  if ((!registryProvidersLoaded || !includeUnset) && selected && selected !== 'shell' && !choices.some((choice) => choice.value === selected)) {
    choices.unshift({ value: selected, label: selected });
  }
  const options = choices.map((choice) => {
    const option = select.ownerDocument.createElement('option');
    option.value = choice.value;
    option.textContent = choice.label || translate(choice.labelKey || '');
    return option;
  });
  select.replaceChildren(...options);
  select.value = choices.some((choice) => choice.value === selected) ? selected : (choices[0]?.value || '');
}

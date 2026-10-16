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

// /api/info contains only usable custom providers. Keep the built-in list above
// as the old-Hub fallback; never infer availability from saved form values.
let customCLIOptions: OrchestrationCLIOption[] = [];
let customProvidersLoaded = false;
const providerChoiceListeners = new Set<() => void>();

export function getOrchestrationCLIOptions(): readonly OrchestrationCLIOption[] {
  return [...ORCHESTRATION_CLI_OPTIONS, ...customCLIOptions];
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
  customProvidersLoaded = true;
  for (const listener of providerChoiceListeners) listener();
}

export function onOrchestrationProvidersChanged(listener: () => void): () => void {
  providerChoiceListeners.add(listener);
  return () => { providerChoiceListeners.delete(listener); };
}

// Use DOM properties, not HTML interpolation: custom labels and ids are data.
// Replacing only options leaves the select, its listeners and the other fields
// intact. A removed role choice falls back to unset; approval requires a CLI.
export function fillOrchestrationProviderSelect(
  select: HTMLSelectElement,
  translate: (key: string) => string,
  selected = select.value,
  includeUnset = true,
): void {
  const choices = getOrchestrationCLIOptions().filter((choice) => includeUnset || choice.value);
  // A confirmation or saved role can arrive before /api/info. Keep its exact
  // provider until the authoritative list arrives instead of silently choosing
  // a built-in. Older Hubs without custom_providers retain this compatibility.
  if (!customProvidersLoaded && selected && selected !== 'shell' && !choices.some((choice) => choice.value === selected)) {
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

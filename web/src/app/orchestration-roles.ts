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

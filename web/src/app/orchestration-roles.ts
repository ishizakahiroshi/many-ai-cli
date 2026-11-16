// Orchestration role choices shared by the spawn settings and relay dialog.
import { declaredProviderIds, providerIsBuiltin, providerLongLabel } from './provider-declarations.js';

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

// 子の役割に選べる同梱の AI CLI（Hub が送る宣言の順。登録した AI CLI は各画面が別に足す）。
// Shell は子の役割に向かないので入れない。
export function orchestrationCliOptions(): readonly OrchestrationCLIOption[] {
  return [
    { value: '', labelKey: 'spawn_role_cli_none' },
    ...declaredProviderIds({ sort: 'hub', includeAcp: false })
      .filter((id) => providerIsBuiltin(id))
      .map((id) => ({ value: id, label: providerLongLabel(id) })),
  ];
}

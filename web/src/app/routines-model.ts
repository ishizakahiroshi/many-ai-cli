export type RoutineSchedule = { kind: 'manual' | 'daily' | 'weekdays'; time: string; timezone: string };
export interface Routine {
  id: string; name: string; cwd: string; provider: string; model: string; prompt: string;
  enabled: boolean; schedule: RoutineSchedule; next_run_at?: string; updated_at?: string; completion_mode?: string;
}
export interface RoutineRun {
  id: string; routine_id: string; routine_name: string; trigger: string; status: string;
  session_id?: number; session_db_id?: number; session_label?: string; hub_instance_id?: string;
  cwd: string; provider: string; model: string; prompt: string;
  started_at: string; updated_at: string; finished_at?: string;
  summary?: string; result?: string; result_truncated?: boolean; result_available: boolean; error?: string;
}
export function isRoutineRunActive(run: Pick<RoutineRun, 'status'>): boolean {
  return ['starting', 'running', 'waiting'].includes(run.status);
}
// 照合は起動時ラベル（launch_label）で行う。label はカード右クリックでいつでも
// 書き換わる表示名なので、改名すると一致しなくなる（子 plan:
// plan_session-card-label-edit.md C1 / C2）。
export function routineSessionMatches(run: RoutineRun, session?: { id: number; launch_label?: string }): boolean {
  return !!session && !!run.session_label && session.id === run.session_id && session.launch_label === run.session_label;
}
export function routineRunFromURL(url: string): string | null {
  const id = new URL(url).searchParams.get('routine_run');
  return id && /^[a-zA-Z0-9_-]{1,100}$/.test(id) ? id : null;
}

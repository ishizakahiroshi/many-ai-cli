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
export function routineSessionMatches(run: RoutineRun, session?: { id: number; label?: string }): boolean {
  return !!session && !!run.session_label && session.id === run.session_id && session.label === run.session_label;
}
export function routineRunFromURL(url: string): string | null {
  const id = new URL(url).searchParams.get('routine_run');
  return id && /^[a-zA-Z0-9_-]{1,100}$/.test(id) ? id : null;
}

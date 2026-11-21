// The local-claude taker (work the board's owner handed to this PC), as
// GET /api/local-taker returns it. The Hub wrote it from this machine's files,
// but titles and summaries came from the board and claude, so each field is
// checked before it is shown.
export interface TakerRun {
  request_id: string;
  item_id: string;
  project: string;
  title: string;
  started_at: number;
  ended_at: number | null;
  /** running / done / failed / interrupted */
  outcome: string;
  summary: string;
}

export interface TakerStatus {
  configured: boolean;
  paused: boolean;
  current: TakerRun | null;
  next_look_at: number | null;
  runs_today: number;
  max_runs_per_day: number;
  interval_secs: number;
  recent: TakerRun[];
}

export interface StreamStep { kind: 'text' | 'tool' | 'tool_result' | 'result' | 'note'; text: string; }

const text = (value: unknown): string => (typeof value === 'string' ? value : '');
const count = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0);
const time = (value: unknown): number | null => (typeof value === 'number' && Number.isFinite(value) ? value : null);

function runFrom(raw: unknown): TakerRun | null {
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  const started = time(row.started_at);
  const run: TakerRun = {
    request_id: text(row.request_id), item_id: text(row.item_id), project: text(row.project),
    title: text(row.title), started_at: started ?? 0, ended_at: time(row.ended_at),
    outcome: text(row.outcome), summary: text(row.summary),
  };
  return run.request_id && started !== null ? run : null;
}

export function takerStatusFrom(payload: unknown): TakerStatus | null {
  const raw = (payload as { taker?: unknown } | null)?.taker;
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  return {
    configured: row.configured === true,
    paused: row.paused === true,
    current: runFrom(row.current),
    next_look_at: time(row.next_look_at),
    runs_today: count(row.runs_today),
    max_runs_per_day: count(row.max_runs_per_day),
    interval_secs: count(row.interval_secs),
    recent: Array.isArray(row.recent) ? row.recent.map(runFrom).filter((r): r is TakerRun => r !== null) : [],
  };
}

const KINDS = new Set(['text', 'tool', 'tool_result', 'result', 'note']);
export function streamStepsFrom(payload: unknown): StreamStep[] {
  const list = (payload as { entries?: unknown } | null)?.entries;
  if (!Array.isArray(list)) return [];
  return list
    .filter((e): e is Record<string, unknown> => !!e && typeof e === 'object')
    .filter(e => KINDS.has(text(e.kind)) && text(e.text) !== '')
    .map(e => ({ kind: text(e.kind) as StreamStep['kind'], text: text(e.text) }));
}

export type TakerNotice =
  | { kind: 'started'; run: TakerRun }
  | { kind: 'finished'; run: TakerRun };

/**
 * What changed between two looks that is worth a desktop notification: a run
 * that started, and a run that ended (done or failed). The first look only
 * learns what is there, so opening the dashboard does not replay old runs.
 */
export function takerNotices(before: TakerStatus | null, after: TakerStatus): TakerNotice[] {
  if (!before) return [];
  const notices: TakerNotice[] = [];
  const seen = new Set(before.recent.map(r => r.request_id));
  const wasGoing = new Set([before.current?.request_id, ...before.recent.filter(r => r.outcome === 'running').map(r => r.request_id)]);
  if (after.current && !wasGoing.has(after.current.request_id)) notices.push({ kind: 'started', run: after.current });
  for (const run of after.recent) {
    if (run.outcome !== 'done' && run.outcome !== 'failed') continue;
    // Ended since the last look: it was going then, or it started and ended in between.
    if (wasGoing.has(run.request_id) || !seen.has(run.request_id)) notices.push({ kind: 'finished', run });
  }
  return notices;
}

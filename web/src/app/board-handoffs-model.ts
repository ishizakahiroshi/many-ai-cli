// Work the board's owner handed over, as GET /api/board-handoffs returns it.
// Everything here came from the board, so each field is checked before it is
// shown or sent back.
export interface HandedWork {
  project: string;
  item_id: string;
  request_id: string;
  title: string;
  requested_at: number | null;
  folder: string;
}

export interface BoardHandoffRef { item_id: string; request_id: string; title: string; }

const text = (value: unknown): string => (typeof value === 'string' ? value : '');

export function handedWorkFrom(payload: unknown): HandedWork[] {
  const list = (payload as { work?: unknown } | null)?.work;
  if (!Array.isArray(list)) return [];
  const found: HandedWork[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    const row = raw as Record<string, unknown>;
    const work: HandedWork = {
      project: text(row.project), item_id: text(row.item_id), request_id: text(row.request_id),
      title: text(row.title), folder: text(row.folder),
      requested_at: typeof row.requested_at === 'number' && Number.isFinite(row.requested_at) ? row.requested_at : null,
    };
    if (work.item_id && work.request_id && work.folder) found.push(work);
  }
  return found;
}

/** The first instruction a session for handed-over work opens with: the title the owner wrote. */
export function handoffPrompt(work: HandedWork): string {
  return work.title.trim();
}
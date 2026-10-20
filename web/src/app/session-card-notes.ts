// Compact-card notes are data only: no DOM, imports, or module-init dependencies.
export interface CardNoteSession {
  permission_mode?: string;
  branch?: string;
  note?: string;
  role?: string;
  orchestration_id?: string;
  execution_mode?: string;
  worktree_branch?: string;
  handoff_from?: number;
  board_notify_pending?: boolean;
}

export interface CardBaselines {
  // null means that there is no unique most frequent value. Empty is a value.
  permissionMode: string | null;
  branch: string | null;
}

export interface CardNote {
  kind: 'permission' | 'branch' | 'note' | 'role' | 'headless' | 'worktree' | 'handoff-from' | 'handoff-to' | 'board';
  labelKey: string;
  value: string;
}

function mostFrequent(values: string[]): string | null {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) || 0) + 1);
  let best: string | null = null;
  let max = 0;
  for (const [value, count] of counts) {
    if (count > max) { best = value; max = count; }
    else if (count === max) best = null;
  }
  return best;
}

/** Pass every session in the project node, including collapsed children. */
export function computeBaselines(sessionsInGroup: readonly CardNoteSession[]): CardBaselines {
  return {
    permissionMode: mostFrequent(sessionsInGroup.map(s => String(s.permission_mode || '').trim())),
    branch: mostFrequent(sessionsInGroup.map(s => String(s.branch || ''))),
  };
}

/** An empty result is the sole rule for omitting the compact card's note dot. */
export function collectCardNotes(session: CardNoteSession, baselines: CardBaselines, successorID = 0): CardNote[] {
  const notes: CardNote[] = [];
  const permission = String(session.permission_mode || '').trim();
  const branch = String(session.branch || '');
  if (baselines.permissionMode === null || permission !== baselines.permissionMode) {
    notes.push({ kind: 'permission', labelKey: 'card_note_permission', value: permission });
  }
  if (baselines.branch === null || branch !== baselines.branch) {
    notes.push({ kind: 'branch', labelKey: 'card_note_branch', value: branch });
  }
  if (session.note) notes.push({ kind: 'note', labelKey: 'card_note_text', value: session.note });
  const role = session.role || (session.orchestration_id ? 'conductor' : '');
  if (role) notes.push({ kind: 'role', labelKey: 'card_note_role', value: role });
  if (session.execution_mode === 'headless') notes.push({ kind: 'headless', labelKey: 'card_headless', value: '' });
  if (session.worktree_branch) notes.push({ kind: 'worktree', labelKey: 'card_note_worktree', value: session.worktree_branch });
  if (session.handoff_from) notes.push({ kind: 'handoff-from', labelKey: 'card_handoff_from_tooltip', value: String(session.handoff_from) });
  if (successorID) notes.push({ kind: 'handoff-to', labelKey: 'card_handoff_to_tooltip', value: String(successorID) });
  if (session.board_notify_pending) notes.push({ kind: 'board', labelKey: 'card_board_update_pending', value: '' });
  return notes;
}

// Versioned, browser-local placement state. Keep this separate from multiPaneOrder:
// that older key always describes the order of *all* live sessions.
import { isValidTabName } from './project-view-memory.js';

export const FLEXIBLE_PANE_STORAGE_KEY = 'flexiblePaneLayoutsV1';
export const PANE_DRAG_MIME = 'application/x-many-ai-cli-pane';

export type PaneContent =
  | { kind: 'session'; sessionId: number; startedAt: string }
  | { kind: 'tab'; tabName: string; sessionId: number | null; startedAt: string | null };

export interface FlexibleLayout {
  cols: number;
  rows: number;
  colFracs: number[];
  rowFracs: number[];
  slots: (PaneContent | null)[];
  /** A drop on a normal view created this split, so the last pane may collapse. */
  autoSplit?: boolean;
}

export type FlexibleLayouts = Record<string, FlexibleLayout>;

/** Grow the grid one row or column at a time within the supported dimensions. */
export function nextPaneDimensions(cols: number, rows: number): { cols: number; rows: number } | null {
  if (rows < 3 && rows < cols) return { cols, rows: rows + 1 };
  if (cols < 6) return { cols: cols + 1, rows };
  if (rows < 3) return { cols, rows: rows + 1 };
  return null;
}

/** Add to an empty slot, growing the grid when necessary without moving existing panes visually. */
export function addPaneContent(layout: FlexibleLayout, content: PaneContent, preferredIdx?: number): number | null {
  const existing = layout.slots.findIndex(slot => slot && paneContentKey(slot) === paneContentKey(content));
  if (existing >= 0) return existing;
  let target = typeof preferredIdx === 'number' && Number.isInteger(preferredIdx) &&
    preferredIdx >= 0 && preferredIdx < layout.slots.length && layout.slots[preferredIdx] === null
      ? preferredIdx : layout.slots.findIndex(slot => slot === null);
  if (target < 0) {
    const next = nextPaneDimensions(layout.cols, layout.rows);
    if (!next) return null;
    const oldCols = layout.cols;
    const oldSlots = layout.slots;
    layout.slots = new Array(next.cols * next.rows).fill(null);
    oldSlots.forEach((slot, idx) => {
      layout.slots[Math.floor(idx / oldCols) * next.cols + idx % oldCols] = slot;
    });
    layout.cols = next.cols;
    layout.rows = next.rows;
    if (layout.colFracs.length < next.cols) layout.colFracs.push(1);
    if (layout.rowFracs.length < next.rows) layout.rowFracs.push(1);
    target = typeof preferredIdx === 'number' && Number.isInteger(preferredIdx) &&
      preferredIdx >= 0 && preferredIdx < layout.slots.length && layout.slots[preferredIdx] === null
        ? preferredIdx : layout.slots.findIndex(slot => slot === null);
  }
  layout.slots[target] = content;
  return target;
}

/** Predict the full grid after adding a pane, while leaving candidate cells empty. */
export function previewPaneDrop(layout: FlexibleLayout, content: PaneContent):
  { layout: FlexibleLayout; suggestedIdx: number; existing: boolean } | null {
  const copy: FlexibleLayout = {
    cols: layout.cols, rows: layout.rows, colFracs: layout.colFracs.slice(),
    rowFracs: layout.rowFracs.slice(), slots: layout.slots.slice(), autoSplit: layout.autoSplit,
  };
  const existing = copy.slots.findIndex(slot => slot && paneContentKey(slot) === paneContentKey(content));
  if (existing >= 0) return { layout: copy, suggestedIdx: existing, existing: true };
  const suggestedIdx = addPaneContent(copy, content);
  if (suggestedIdx === null) return null;
  copy.slots[suggestedIdx] = null;
  return { layout: copy, suggestedIdx, existing: false };
}

export function paneLayoutKey(scope: unknown, projectKey: unknown): string {
  return scope === 'box' && typeof projectKey === 'string' && projectKey.length > 0
    ? `box:${projectKey}` : 'all';
}

/** A session's related views stay separate from the cross-session overview. */
export function sessionPaneLayoutKey(sessionId: number, startedAt: string): string {
  return `session:${sessionId}:${startedAt}`;
}

export function paneContentKey(content: PaneContent): string {
  return content.kind === 'session'
    ? `session:${content.sessionId}:${content.startedAt}`
    : ['chat', 'approval', 'history', 'orchestration'].includes(content.tabName)
      ? `tab:${content.tabName}`
      : `tab:${content.tabName}:${content.sessionId ?? ''}:${content.startedAt ?? ''}`;
}

export function normalizePaneContent(raw: unknown): PaneContent | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  if (value.kind === 'session') {
    if (typeof value.sessionId !== 'number' || !Number.isSafeInteger(value.sessionId) || value.sessionId <= 0 ||
        typeof value.startedAt !== 'string' || !value.startedAt) return null;
    return { kind: 'session', sessionId: Number(value.sessionId), startedAt: value.startedAt };
  }
  if (value.kind !== 'tab' || !isValidTabName(value.tabName) ||
      value.tabName === 'multi' || value.tabName === 'terminal') return null;
  const sessionId = value.sessionId == null ? null : Number(value.sessionId);
  if (sessionId !== null && (!Number.isSafeInteger(sessionId) || sessionId <= 0)) return null;
  const startedAt = sessionId === null ? null : value.startedAt;
  if (sessionId !== null && (typeof startedAt !== 'string' || !startedAt)) return null;
  return { kind: 'tab', tabName: String(value.tabName), sessionId,
    startedAt: startedAt as string | null };
}

export function normalizeFlexibleLayout(raw: unknown): FlexibleLayout | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const cols = Number(value.cols);
  const rows = Number(value.rows);
  if (!Number.isInteger(cols) || cols < 1 || cols > 6 ||
      !Number.isInteger(rows) || rows < 1 || rows > 3 || !Array.isArray(value.slots)) return null;
  const frac = (rawFracs: unknown, count: number): number[] =>
    Array.isArray(rawFracs) && rawFracs.length === count &&
    rawFracs.every(v => typeof v === 'number' && Number.isFinite(v) && v > 0)
      ? rawFracs.slice() : new Array(count).fill(1);
  const seen = new Set<string>();
  const slots = Array.from({ length: cols * rows }, (_, i) => {
    const content = normalizePaneContent(value.slots[i]);
    if (!content) return null;
    const key = paneContentKey(content);
    if (seen.has(key)) return null;
    seen.add(key);
    return content;
  });
  return { cols, rows, colFracs: frac(value.colFracs, cols), rowFracs: frac(value.rowFracs, rows),
    slots, autoSplit: value.autoSplit === true };
}

export function normalizeFlexibleLayouts(raw: unknown): FlexibleLayouts {
  const value = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {};
  const result: FlexibleLayouts = {};
  for (const [key, entry] of Object.entries(value).slice(0, 201)) {
    const sessionKey = /^session:([1-9]\d*):(.{1,256})$/.exec(key);
    if (key !== 'all' && (!key.startsWith('box:') || key.length > 516) && !sessionKey) continue;
    const layout = normalizeFlexibleLayout(entry);
    if (!layout) continue;
    if (sessionKey) {
      const id = Number(sessionKey[1]);
      if (!Number.isSafeInteger(id)) continue;
      // Reject foreign-session content in a saved workspace, including malformed old data.
      layout.slots = layout.slots.map(slot => slot?.sessionId === id &&
        slot.startedAt === sessionKey[2] ? slot : null);
    }
    result[key] = layout;
  }
  return result;
}

/** Move previously mixed session tabs into their owners' workspaces once. */
export function migrateSessionTabsToWorkspaces(layouts: FlexibleLayouts): boolean {
  let changed = false;
  for (const [key, overview] of Object.entries(layouts)) {
    if (key !== 'all' && !key.startsWith('box:')) continue;
    overview.slots.forEach((slot, idx) => {
      if (!slot || slot.kind !== 'tab' || slot.sessionId == null || !slot.startedAt) return;
      const workspaceKey = sessionPaneLayoutKey(slot.sessionId, slot.startedAt);
      let workspace = layouts[workspaceKey];
      if (!workspace) {
        workspace = { cols: 2, rows: 1, colFracs: [1, 1], rowFracs: [1],
          slots: [{ kind: 'session', sessionId: slot.sessionId, startedAt: slot.startedAt }, null],
          autoSplit: true };
        layouts[workspaceKey] = workspace;
      }
      if (addPaneContent(workspace, slot) === null) return;
      overview.slots[idx] = null;
      changed = true;
    });
  }
  return changed;
}

export function parsePaneDragPayload(raw: string):
  { kind: 'session'; sessionId: number } |
  { kind: 'tab'; tabName: string; sessionId: number | null } | null {
  try {
    const value = JSON.parse(raw);
    if (!value || typeof value !== 'object') return null;
    const id = value.sessionId == null ? null : Number(value.sessionId);
    if (id !== null && (!Number.isSafeInteger(id) || id <= 0)) return null;
    if (value.kind === 'session') return id === null ? null : { kind: 'session', sessionId: id };
    if (value.kind === 'tab' && isValidTabName(value.tabName) &&
        value.tabName !== 'multi') {
      return { kind: 'tab', tabName: value.tabName, sessionId: id };
    }
  } catch (_) { /* External drags are not placement requests. */ }
  return null;
}

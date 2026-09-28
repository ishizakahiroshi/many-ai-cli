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

export function paneLayoutKey(scope: unknown, projectKey: unknown): string {
  return scope === 'box' && typeof projectKey === 'string' && projectKey.length > 0
    ? `box:${projectKey}` : 'all';
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
      value.tabName === 'multi' || value.tabName === 'split' || value.tabName === 'terminal') return null;
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
  for (const [key, entry] of Object.entries(value).slice(0, 101)) {
    if (key !== 'all' && (!key.startsWith('box:') || key.length > 516)) continue;
    const layout = normalizeFlexibleLayout(entry);
    if (layout) result[key] = layout;
  }
  return result;
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
        value.tabName !== 'multi' && value.tabName !== 'split') {
      return { kind: 'tab', tabName: value.tabName, sessionId: id };
    }
  } catch (_) { /* External drags are not placement requests. */ }
  return null;
}

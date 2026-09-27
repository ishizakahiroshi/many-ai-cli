// Terminal identity, rather than a reusable session number, owns its viewport.
// No imports from app/state: callers provide the terminal and control layout.
export interface TerminalViewEntry {
  autoScroll?: boolean;
  term?: {
    buffer: { active: { viewportY: number; baseY: number; type?: string } };
    scrollToLine(line: number): void;
    scrollToBottom(): void;
  };
}
interface TerminalPosition { line: number; following: boolean; bufferType?: string }
const positions = new WeakMap<object, TerminalPosition>();

export function captureTerminalView(entry: TerminalViewEntry | null | undefined): void {
  if (!entry?.term?.buffer?.active) return;
  const buffer = entry.term.buffer.active;
  positions.set(entry, {
    line: Math.max(0, buffer.viewportY),
    following: entry.autoScroll !== false && buffer.viewportY >= buffer.baseY,
    bufferType: buffer.type,
  });
}

export function hasTerminalView(entry: TerminalViewEntry | null | undefined): boolean {
  return !!entry && positions.has(entry);
}

export function terminalViewFollows(entry: TerminalViewEntry | null | undefined): boolean {
  return !entry || positions.get(entry)?.following !== false;
}

export function restoreTerminalView(entry: TerminalViewEntry | null | undefined): boolean {
  if (!entry?.term?.buffer?.active) return false;
  const saved = positions.get(entry);
  if (!saved) return false;
  const buffer = entry.term.buffer.active;
  // Alternate-screen buffers have no scrollback; an old normal-buffer line is invalid there.
  if (saved.bufferType !== buffer.type || buffer.type === 'alternate') {
    entry.autoScroll = true;
    entry.term.scrollToBottom();
    return true;
  }
  entry.autoScroll = saved.following;
  if (saved.following) entry.term.scrollToBottom();
  else entry.term.scrollToLine(Math.min(saved.line, Math.max(0, buffer.baseY)));
  return true;
}

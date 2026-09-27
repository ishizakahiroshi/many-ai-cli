import { describe, expect, test } from 'bun:test';
import { captureTerminalView, hasTerminalView, restoreTerminalView, terminalViewFollows } from '../src/app/session-view-position';

function terminal(line = 25, base = 100) {
  const active = { viewportY: line, baseY: base, type: 'normal' };
  return { autoScroll: line === base, term: { buffer: { active }, scrollToLine(value: number) { active.viewportY = value; }, scrollToBottom() { active.viewportY = active.baseY; } } };
}
describe('terminal view position', () => {
  test('restores history position while new output arrives', () => {
    const entry = terminal(); captureTerminalView(entry);
    entry.term.buffer.active.baseY = 150; entry.term.scrollToBottom(); entry.autoScroll = true;
    expect(terminalViewFollows(entry)).toBe(false);
    expect(restoreTerminalView(entry)).toBe(true);
    expect(entry.term.buffer.active.viewportY).toBe(25);
    expect(entry.autoScroll).toBe(false);
  });
  test('follows latest only when previously following', () => {
    const entry = terminal(100); captureTerminalView(entry); entry.term.buffer.active.baseY = 140;
    restoreTerminalView(entry); expect(entry.term.buffer.active.viewportY).toBe(140); expect(entry.autoScroll).toBe(true);
  });
  test('new terminal instance does not inherit a reused session position', () => {
    const old = terminal(); captureTerminalView(old);
    const replacement = terminal(); expect(hasTerminalView(replacement)).toBe(false); expect(restoreTerminalView(replacement)).toBe(false);
  });
  test('truncated scrollback and alternate buffer cannot restore an invalid line', () => {
    const entry = terminal(); captureTerminalView(entry); entry.term.buffer.active.baseY = 10;
    restoreTerminalView(entry); expect(entry.term.buffer.active.viewportY).toBe(10);
    entry.term.buffer.active.type = 'alternate'; restoreTerminalView(entry); expect(entry.autoScroll).toBe(true);
  });
});

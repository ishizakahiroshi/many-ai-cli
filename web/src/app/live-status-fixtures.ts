import assert from 'node:assert/strict';
import test from 'node:test';
import {
  extractCodexLiveStatusFromLines,
  extractCopilotLiveStatusFromLines,
  extractCursorAgentLiveStatusFromLines,
} from './live-status.js';

test('live status: Codex prefers the Working status line', () => {
  const lines = [
    '• Running rg -n liveStatus web/src/app/terminal.ts',
    '• Working (2m 22s • esc to interrupt)',
    '› Run /review on my current changes',
  ];
  assert.equal(extractCodexLiveStatusFromLines(lines), 'Working (2m 22s • esc to interrupt)');
});

test('live status: Codex falls back to the latest action line', () => {
  const lines = [
    'noise',
    '• Reading web/src/app/terminal.ts',
    '• Ran bun run check',
  ];
  assert.equal(extractCodexLiveStatusFromLines(lines), 'Ran bun run check');
});

test('live status: Copilot extracts Working and model from the status row', () => {
  const lines = [
    '/ commands · ? help',
    ' ● Working esc cancel                                                                                        Auto → claude-haiku-4.5',
  ];
  assert.equal(extractCopilotLiveStatusFromLines(lines), 'Working esc cancel · Auto → claude-haiku-4.5');
});

test('live status: Copilot ignores prompt text glued after the status', () => {
  const lines = [
    '今日は 何日かな @ files · # issues / commands · ? help ● Working esc cancel❯ 今日は 何日かな',
  ];
  assert.equal(extractCopilotLiveStatusFromLines(lines), 'Working esc cancel');
});

test('live status: Cursor Agent combines follow-up state with usage percent', () => {
  const lines = [
    '  → Add a follow-up        ctrl+c to stop',
    '',
    '  Auto · 12.1%',
    '  C:\\dev\\github\\public\\many-ai-cli · develop',
  ];
  assert.equal(extractCursorAgentLiveStatusFromLines(lines), 'Add a follow-up · Auto · 12.1%');
});

test('live status: Cursor Agent can return only the usage percent', () => {
  const lines = [
    '  Tip: Use /config to customize Cursor settings and behavior.',
    '  Auto · 14.6%',
  ];
  assert.equal(extractCursorAgentLiveStatusFromLines(lines), 'Auto · 14.6%');
});

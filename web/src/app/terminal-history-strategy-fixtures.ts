import assert from 'node:assert/strict';
import test from 'node:test';

import {
  isConfirmedAltScreenChange,
  resolveTerminalHistoryStrategy,
  terminalHistoryCapabilitiesForProvider,
  type TerminalScreenSnapshot,
} from './terminal-history-strategy.js';

test('terminal-history: main buffer は Provider に関係なく native scrollback', () => {
  const grok = terminalHistoryCapabilitiesForProvider('grok');
  assert.equal(resolveTerminalHistoryStrategy('normal', grok, true), 'native-scrollback');
});

test('terminal-history: main buffer に履歴が無ければ宣言済み transcript viewer へ倒す', () => {
  const grok = terminalHistoryCapabilitiesForProvider('grok');
  assert.equal(resolveTerminalHistoryStrategy('normal', grok, false), 'transcript-viewer');
});

test('terminal-history: 未知 Provider の alternate buffer は confirmed input', () => {
  const unknown = terminalHistoryCapabilitiesForProvider('future-ai');
  assert.deepEqual(unknown, { altInput: 'auto' });
  assert.equal(resolveTerminalHistoryStrategy('alternate', unknown), 'alt-input-confirmed');
});

test('terminal-history: 専用履歴 surface は transcript viewer を選ぶ', () => {
  const grok = terminalHistoryCapabilitiesForProvider('GROK');
  assert.equal(grok.transcriptViewer, 'grok-chat');
  assert.equal(resolveTerminalHistoryStrategy('alternate', grok), 'transcript-viewer');
});

test('terminal-history: 安全な履歴 capability が無ければ none', () => {
  assert.equal(resolveTerminalHistoryStrategy('alternate', { altInput: 'none' }), 'none');
  assert.equal(resolveTerminalHistoryStrategy('unknown', { altInput: 'auto' }), 'none');
});

function screen(lines: string[], overrides: Partial<TerminalScreenSnapshot> = {}): TerminalScreenSnapshot {
  return { bufferType: 'alternate', cols: 80, rows: lines.length, lines, ...overrides };
}

test('terminal-history: PTY 応答後も本文が同じなら空振り', () => {
  assert.equal(isConfirmedAltScreenChange(screen(['old', 'latest']), screen(['old', 'latest']), -1), false);
});

test('terminal-history: alternate 本文が上方向へずれた分だけ確定', () => {
  const before = screen(['line-a', 'line-b', 'line-c', 'line-d']);
  const after = screen(['older-a', 'older-b', 'line-a', 'line-b']);
  assert.equal(isConfirmedAltScreenChange(before, after, -1), true);
});

test('terminal-history: alternate 本文が下方向へずれた分だけ確定', () => {
  const before = screen(['older-a', 'older-b', 'line-a', 'line-b']);
  const after = screen(['line-a', 'line-b', 'line-c', 'line-d']);
  assert.equal(isConfirmedAltScreenChange(before, after, 1), true);
});

test('terminal-history: status 行だけの再描画はスクロール成功にしない', () => {
  const before = screen(['header', 'line-a', 'line-b', 'spinner-1']);
  const after = screen(['header', 'line-a', 'line-b', 'spinner-2']);
  assert.equal(isConfirmedAltScreenChange(before, after, -1), false);
});

test('terminal-history: resize と buffer 遷移はスクロール成功にしない', () => {
  const before = screen(['old', 'latest']);
  assert.equal(isConfirmedAltScreenChange(before, screen(['older', 'old'], { cols: 100 }), -1), false);
  assert.equal(isConfirmedAltScreenChange(before, screen(['older', 'old'], { bufferType: 'normal' }), -1), false);
});

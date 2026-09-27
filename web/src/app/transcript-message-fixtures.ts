import assert from 'node:assert/strict';
import test from 'node:test';
import { transcriptMessageIdentity, transcriptMessageKey, transcriptMessagePresentation, transcriptReadPosition, transcriptRestoreTop } from './transcript-message.js';

test('tool result updates keep the same transcript identity', () => {
  const call = {
    role: 'assistant',
    kind: 'tool',
    ts: '2026-08-11T00:00:01Z',
    message_id: 'tool:call-1',
    tools: [{ id: 'call-1', name: 'shell', input: 'pwd' }],
  };
  const result = {
    ...call,
    tools: [{ ...call.tools[0], result: 'C:/work' }],
  };
  assert.equal(transcriptMessageIdentity(call), transcriptMessageIdentity(result));
  assert.notEqual(transcriptMessageKey(call), transcriptMessageKey(result));
});

test('chat excludes thought-only and sidechain records while preserving answer text with attached thoughts', () => {
  const meta = { transcript: true, thinking: ['Synthetic private reasoning'] };
  assert.equal(transcriptMessagePresentation({ role: 'ai', kind: 'thinking', meta }), 'hidden');
  assert.equal(transcriptMessagePresentation({ role: 'ai', kind: 'sidechain', meta }), 'hidden');
  assert.equal(transcriptMessagePresentation({ role: 'ai', kind: 'text', rawText: 'Synthetic answer', meta }, 'claude'), 'answer');
  assert.equal(transcriptMessagePresentation({ role: 'ai', kind: 'text', rawText: 'Synthetic answer', meta }, 'command-code'), 'answer');
});

test('Codex final and commentary use metadata; legacy or future text is retained for explicit disclosure', () => {
  const msg = { role: 'ai', kind: 'text', rawText: 'The work is done.', meta: { transcript: true } };
  assert.equal(transcriptMessagePresentation(msg, 'codex'), 'unclassified');
  for (const presentation of ['answer', 'progress', 'hidden', 'unclassified'] as const) {
    assert.equal(transcriptMessagePresentation({ ...msg, meta: { ...msg.meta, presentation } }, 'codex'), presentation);
  }
  assert.equal(msg.rawText, 'The work is done.');
  assert.equal(transcriptMessagePresentation({ ...msg, role: 'user' }, 'codex'), 'answer');
  assert.notEqual(transcriptMessageKey({ text: 'same', presentation: 'progress' }), transcriptMessageKey({ text: 'same', presentation: 'answer' }));
});

test('thought-free tool records remain reachable and empty transcript records create no bubble', () => {
  const msg = { role: 'ai', kind: 'tool', meta: { transcript: true, tools: [{ name: 'read', result: 'Synthetic file' }] } };
  assert.equal(transcriptMessagePresentation(msg, 'codex'), 'tool');
  assert.equal(transcriptMessagePresentation({ ...msg, meta: { transcript: true } }, 'codex'), 'hidden');
});

test('returning to a reading position preserves the reader; tail followers see appended output', () => {
  const reading = transcriptReadPosition(140, 2000, 600);
  const tail = transcriptReadPosition(1400, 2000, 600);
  assert.equal(transcriptRestoreTop(reading, 2400, 600), 140);
  assert.equal(transcriptRestoreTop(tail, 2400, 600), 1800);
  assert.equal(transcriptRestoreTop(undefined, 2400, 600), 1800);
  assert.equal(transcriptRestoreTop(reading, 100, 600), 0);
});

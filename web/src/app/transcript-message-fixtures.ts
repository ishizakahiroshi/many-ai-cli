import assert from 'node:assert/strict';
import test from 'node:test';
import { transcriptMessageIdentity, transcriptMessageKey } from './transcript-message.js';

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

// agent-session-model.ts を、疑似サーバー（Hub の C11 の events の形）の流れを再現して固定する。
// 合成データだけ。
import assert from 'node:assert/strict';
import test from 'node:test';
import { applyEventsResponse, diffLines, emptyState, permissionAnswer } from './agent-session-model.js';
import type { AgentEvent, PendingPermission } from './agent-session-model.js';

let seq = 0;
const ev = (kind: string, data: Record<string, any>): AgentEvent => ({ seq: ++seq, at: '2026-01-01T00:00:00Z', kind, data });
const update = (value: Record<string, any>, replay = false): AgentEvent => ev('update', { session_id: 'a1', update: value, ...(replay ? { replay: true } : {}) });
const chunk = (text: string, kind = 'agent_message_chunk') => update({ sessionUpdate: kind, content: { type: 'text', text } });
const pendingOne = (key = 7): PendingPermission => ({
  request_key: key,
  tool_call: { toolCallId: 't1', title: 'Write file' },
  options: [
    { option_id: 'allow', name: 'Allow once', kind: 'allow_once' },
    { option_id: 'deny', name: 'Reject', kind: 'reject_once' },
  ],
  at: '2026-01-01T00:00:00Z',
});
const apply = (state = emptyState(), events: AgentEvent[] = [], pending: PendingPermission[] = [], phase = 'running', first = 1) =>
  applyEventsResponse(state, { session: { phase }, first_seq: first, events, pending_permissions: pending }, state.lastSeq);

test('streaming chunks of one kind join into one bubble; a prompt and a reply stay separate', () => {
  seq = 0;
  const state = apply(emptyState(), [ev('user_prompt', { text: 'hi' }), chunk('Hel'), chunk('lo'), chunk(' there')]);
  assert.deepEqual(state.items.map((i) => i.type), ['user', 'agent']);
  assert.equal((state.items[1] as { text: string }).text, 'Hello there');
  assert.equal(state.lastSeq, 4);
});

test('the same response applied twice does not add anything', () => {
  seq = 0;
  const events = [chunk('a'), chunk('b')];
  const once = apply(emptyState(), events);
  const twice = applyEventsResponse(once, { session: { phase: 'running' }, first_seq: 1, events, pending_permissions: [] }, 0);
  assert.equal(twice.items.length, once.items.length);
  assert.equal(twice.lastSeq, once.lastSeq);
});

test('a tool call is created, updated by id, and carries a diff and text', () => {
  seq = 0;
  const state = apply(emptyState(), [
    update({ sessionUpdate: 'tool_call', toolCallId: 't1', title: 'Edit a.txt', kind: 'edit', status: 'pending' }),
    update({
      sessionUpdate: 'tool_call_update', toolCallId: 't1', status: 'completed',
      content: [{ type: 'diff', path: '/w/a.txt', oldText: 'one\ntwo', newText: 'one\nTWO' }, { type: 'content', content: { type: 'text', text: 'saved' } }],
      locations: [{ path: '/w/a.txt' }],
    }),
  ]);
  assert.equal(state.items.length, 1);
  const tool = state.items[0];
  assert.equal(tool.type, 'tool');
  if (tool.type !== 'tool') return;
  assert.equal(tool.status, 'completed');
  assert.equal(tool.title, 'Edit a.txt');
  assert.deepEqual(tool.diffs, [{ path: '/w/a.txt', oldText: 'one\ntwo', newText: 'one\nTWO' }]);
  assert.deepEqual(tool.texts, ['saved']);
  assert.deepEqual(tool.locations, ['/w/a.txt']);
});

test('an update for an unknown tool call is ignored, an unknown status is shown as unknown', () => {
  seq = 0;
  let state = apply(emptyState(), [update({ sessionUpdate: 'tool_call_update', toolCallId: 'nope', status: 'completed' })]);
  assert.equal(state.items.length, 0);
  state = apply(state, [update({ sessionUpdate: 'tool_call', toolCallId: 'x', title: 'T', status: 'weird' })]);
  assert.equal((state.items[0] as { status: string }).status, 'unknown');
});

test('permission buttons come only from the response pending_permissions, never from the events', () => {
  seq = 0;
  const open = ev('permission_request', { request_key: 7, tool_call: { title: 'Write file' }, options: pendingOne().options });
  // 出来事に許可の要求があっても、保留中の記録が空なら、ボタンの材料は空。
  let state = apply(emptyState(), [open], []);
  assert.equal(state.pending.length, 0);
  assert.equal(state.items.some((i) => i.type === 'permission' && i.state === 'open'), true);
  // 保留中の記録が来て初めて材料ができる。
  state = apply(state, [], [pendingOne()]);
  assert.deepEqual(state.pending.map((p) => p.request_key), [7]);
  assert.deepEqual(state.pending[0].options.map((o) => o.option_id), ['allow', 'deny']);
  // 答えの出来事が来て保留が空になると、材料は消え、並びの表示は answered になる。
  state = apply(state, [ev('permission_answered', { request_key: 7, answer: { option_id: 'allow' }, by: 'dashboard' })], []);
  assert.equal(state.pending.length, 0);
  const item = state.items.find((i) => i.type === 'permission');
  assert.ok(item && item.type === 'permission' && item.state === 'answered' && item.chosen === 'allow');
});

test('closing a permission by cancel or by the session ending is shown as closed', () => {
  seq = 0;
  const state = apply(emptyState(), [
    ev('permission_request', { request_key: 1, tool_call: { title: 'A' }, options: [] }),
    ev('permission_closed', { request_key: 1, reason: 'cancelled' }),
  ]);
  const item = state.items.find((i) => i.type === 'permission');
  assert.ok(item && item.type === 'permission' && item.state === 'closed' && item.reason === 'cancelled');
});

test('the answer bodies are exactly {request_key, option_id} or {request_key, cancelled:true}', () => {
  assert.deepEqual(permissionAnswer(7, { optionId: 'allow' }), { request_key: 7, option_id: 'allow' });
  assert.deepEqual(permissionAnswer(7, { cancelled: true }), { request_key: 7, cancelled: true });
});

test('replayed history is marked; a turn end, an error and an end are kept in order', () => {
  seq = 0;
  const state = apply(emptyState(), [
    update({ sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'old question' } }, true),
    update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'old answer' } }, true),
    ev('turn_end', { stop_reason: 'end_turn' }),
    ev('turn_error', { message: 'boom' }),
    ev('ended', { reason: 'ended by the person' }),
  ], [], 'ended');
  assert.deepEqual(state.items.map((i) => i.type), ['user', 'agent', 'turn_end', 'error', 'ended']);
  assert.equal((state.items[0] as { replay: boolean }).replay, true);
  assert.equal(state.phase, 'ended');
});

test('plan updates replace the previous plan in place', () => {
  seq = 0;
  const state = apply(emptyState(), [
    update({ sessionUpdate: 'plan', entries: [{ content: 'a', status: 'pending' }] }),
    update({ sessionUpdate: 'plan', entries: [{ content: 'a', status: 'completed' }, { content: 'b', status: 'pending' }] }),
  ]);
  assert.equal(state.items.length, 1);
  assert.equal((state.items[0] as { entries: unknown[] }).entries.length, 2);
});

test('the model is display-only and comes from the agent; usage is only stored, never requested', () => {
  seq = 0;
  const state = apply(emptyState(), [
    update({ sessionUpdate: 'config_option_update', configOptions: [{ id: 'model', category: 'model', currentValue: 'model-x' }, { id: 'mode', currentValue: 'm' }] }),
    update({ sessionUpdate: 'usage_update', used: 120, size: 1000 }),
  ]);
  assert.equal(state.model, 'model-x');
  assert.deepEqual(state.usage, { used: 120, size: 1000, cost: undefined });
  assert.equal(state.items.length, 0);
});

test('a gap is reported when the memory no longer holds the events right after the requested one', () => {
  seq = 0;
  const state = applyEventsResponse(emptyState(), { session: { phase: 'idle' }, first_seq: 50, events: [], pending_permissions: [] }, 0);
  assert.equal(state.gap, true);
  const none = applyEventsResponse(emptyState(), { session: { phase: 'idle' }, first_seq: 1, events: [], pending_permissions: [] }, 0);
  assert.equal(none.gap, false);
});

test('malformed events and unknown kinds are skipped without throwing', () => {
  const state = apply(emptyState(), [
    { seq: 9001, at: '', kind: 'meta', data: { provider: 'x' } },
    { seq: 9002, at: '', kind: 'notification', data: { method: 'foo' } },
    { seq: 9003, at: '', kind: 'update', data: { update: { sessionUpdate: 'future_kind' } } },
    { seq: 9004, at: '', kind: 'update', data: {} },
  ]);
  assert.equal(state.items.length, 0);
  assert.equal(state.lastSeq, 9004);
});

test('diffLines folds identical head and tail and keeps a few lines of context', () => {
  const before = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'].join('\n');
  const after = ['a', 'b', 'c', 'd', 'E', 'f', 'g', 'h'].join('\n');
  const lines = diffLines(before, after, 1);
  assert.deepEqual(lines.map((l) => l.op + l.text), ['…', ' d', '-e', '+E', ' f', '…']);
  assert.deepEqual(diffLines('', 'x\ny').map((l) => l.op), ['+', '+']);
  assert.deepEqual(diffLines('x', '').map((l) => l.op), ['-']);
});

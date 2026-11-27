import test from 'node:test';
import assert from 'node:assert/strict';
import { approvalHistoryTarget, findQuestionMessageIndex } from '../src/app/approval-history-target.js';

const session = { provider: 'claude', started_at: '2026-01-02T00:00:00Z' };
const row = {
  state: 'pending', provider: 'claude', candidate_key: 'k1', source_epoch: 2,
  detected_at: '2026-01-02T00:03:00Z',
};
const record = { candidate_key: 'k1', source_epoch: 2 };

test('いまも待っている承認は回答パネルへ', () => {
  assert.equal(approvalHistoryTarget(row, session, record), 'current');
});

test('記録が別の承認に替わった行・回答済みの行は、そのセッションのチャットへ', () => {
  assert.equal(approvalHistoryTarget(row, session, null), 'past');
  assert.equal(approvalHistoryTarget(row, session, { candidate_key: 'k2', source_epoch: 3 }), 'past');
  assert.equal(approvalHistoryTarget({ ...row, state: 'resolved' }, session, record), 'past');
});

test('同じ番号の別のセッションへは移動しない', () => {
  assert.equal(approvalHistoryTarget(row, undefined, null), 'gone');
  assert.equal(approvalHistoryTarget(row, { ...session, started_at: '2026-01-03T00:00:00Z' }, null), 'gone');
  assert.equal(approvalHistoryTarget(row, { ...session, provider: 'codex' }, null), 'gone');
  assert.equal(approvalHistoryTarget(row, { provider: 'claude' }, null), 'gone');
});

test('質問を含む発言を、空白と改行の違いを無視して探す', () => {
  const messages = [
    { role: 'ai', ts: '2026-01-02T00:01:00Z', rawText: 'ほかの発言' },
    { role: 'ai', ts: '2026-01-02T00:03:00Z', rawText: '前置き\n進めて\nいいですか？' },
    { role: 'user', ts: '2026-01-02T00:03:00Z', rawText: '進めていいですか？' },
    { role: 'ai', ts: '2026-01-02T05:00:00Z', rawText: '進めていいですか？' },
  ];
  assert.equal(findQuestionMessageIndex(messages, '進めて いいですか？', row.detected_at), 1);
  assert.equal(findQuestionMessageIndex(messages, '進めていいですか？', ''), 3);
  assert.equal(findQuestionMessageIndex(messages, '無い質問', row.detected_at), -1);
  assert.equal(findQuestionMessageIndex(messages, '', row.detected_at), -1);
});

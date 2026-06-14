import assert from 'node:assert/strict';
import test from 'node:test';
import { parseWorkflowProgress } from './workflow-progress.js';

// ⚠ これらの fixture は実機 1 サンプルが取れるまでの「想定フォーマット」。
// CLI の Workflow 表示が判明したら CALIBRATE 箇所（workflow-progress.ts のグリフ／
// 見出し定義）とこの fixture を合わせて更新する。

test('parseWorkflowProgress: 走行中のブロックを構造化する', () => {
  const lines = [
    'Some unrelated terminal output above',
    '⚙ workflow review-changes',
    '  ▸ Review',
    '    ✓ review:bugs',
    '    ⠋ review:perf',
    '  ▸ Verify',
    '    ⠹ verify:foo.ts',
    '  3 agents · 1 done',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, true);
  assert.equal(r.running, true);
  assert.equal(r.name, 'review-changes');
  assert.equal(r.totalCount, 3);
  assert.equal(r.doneCount, 1);
  assert.equal(r.runningCount, 2);
  assert.equal(r.phases.length, 2);
  assert.equal(r.phases[0].title, 'Review');
  assert.deepEqual(r.phases[0].agents.map(a => a.label), ['review:bugs', 'review:perf']);
  assert.equal(r.phases[0].agents[0].state, 'done');
  assert.equal(r.phases[0].agents[1].state, 'running');
  assert.equal(r.phases[1].title, 'Verify');
});

test('parseWorkflowProgress: 全完了は detected:true / running:false', () => {
  const lines = [
    '⚙ workflow find-flaky-tests',
    '  ✓ scan:logs',
    '  ✓ fix:test-a',
    '  ✓ fix:test-b',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, true);
  assert.equal(r.running, false);
  assert.equal(r.totalCount, 3);
  assert.equal(r.doneCount, 3);
  assert.equal(r.percent, 100);
});

test('parseWorkflowProgress: 明示の % を優先採用する', () => {
  const lines = [
    '⚙ workflow migrate',
    '  ⠋ migrate:a.ts',
    '  ○ migrate:b.ts',
    '  Progress: 40%',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, true);
  assert.equal(r.running, true);
  assert.equal(r.percent, 40);
});

test('parseWorkflowProgress: 非 Workflow バッファは detected:false（誤検出しない）', () => {
  const lines = [
    'Here is a list of options:',
    '1. First option',
    '2. Second option',
    '✓ done editing the file',
    'Anything else?',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, false);
  assert.equal(r.running, false);
});

test('parseWorkflowProgress: 空・null は detected:false', () => {
  assert.equal(parseWorkflowProgress([]).detected, false);
  assert.equal(parseWorkflowProgress(undefined as unknown as string[]).detected, false);
});

test('parseWorkflowProgress: 見出しのみでエージェント行が無ければ detected:false', () => {
  const r = parseWorkflowProgress(['⚙ workflow empty-run', 'starting...']);
  assert.equal(r.detected, false);
});

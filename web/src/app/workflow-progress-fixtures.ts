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

test('parseWorkflowProgress: 背景実行の「N/M agents done」サマリーを権威として走行判定する', () => {
  // ⚙ 見出しが無く（背景実行でツリーが出ない）、ステータス行だけが残るケース。
  const r = parseWorkflowProgress([
    'some output',
    '◑ ur… 43/90 agents done · 5m 25s · ↓ 1.9m',
  ]);
  assert.equal(r.detected, true);
  assert.equal(r.running, true);
  assert.equal(r.live, true); // done<total の生サマリー＝凍結 settle 禁止
  assert.equal(r.totalCount, 90);
  assert.equal(r.doneCount, 43);
  assert.equal(r.runningCount, 47);
  assert.equal(r.percent, 48);
});

test('parseWorkflowProgress: サマリーは経過時間込みで frameSig が変わる（凍結誤判定防止）', () => {
  const a = parseWorkflowProgress(['◑ wf 43/90 agents done · 5m 25s']);
  const b = parseWorkflowProgress(['◑ wf 43/90 agents done · 5m 26s']); // 経過時間だけ進む
  assert.equal(a.runningCount, b.runningCount); // 件数は同じでも
  assert.notEqual(a.frameSig, b.frameSig);      // frameSig は変わる→凍結しない
});

test('parseWorkflowProgress: N/N agents done は完了（live:false）', () => {
  const r = parseWorkflowProgress(['✓ wf 90/90 agents done · 9m 2s']);
  assert.equal(r.detected, true);
  assert.equal(r.running, false);
  assert.equal(r.live, false);
  assert.equal(r.percent, 100);
});

test('parseWorkflowProgress: frameSig は生グリフを含み、同フレームで安定・別グリフで変化する', () => {
  const base = ['⚙ workflow w', '  ▸ P', '    ⠋ a', '    ⠹ b'];
  const r1 = parseWorkflowProgress(base);
  const r2 = parseWorkflowProgress(base);
  // 同一フレームは frameSig 一致（凍結検出が連続一致を数えられる）。
  assert.equal(r1.frameSig, r2.frameSig);
  // スピナーが回った（グリフが変わった）フレームは frameSig が変化する。
  const r3 = parseWorkflowProgress(['⚙ workflow w', '  ▸ P', '    ⠙ a', '    ⠸ b']);
  assert.notEqual(r1.frameSig, r3.frameSig);
  // 状態（running 件数）自体は同じでも frameSig だけが変わる＝「生きている」検知に使える。
  assert.equal(r1.runningCount, r3.runningCount);
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

test('parseWorkflowProgress: waitingDynamic は背景 Workflow 件数を拾う', () => {
  const r = parseWorkflowProgress(['Waiting for 2 dynamic workflows to finish']);
  assert.equal(r.waitingDynamic, 2);
});

test('parseWorkflowProgress: waitingDynamic は行頭 * 付き単数形も拾う', () => {
  const r = parseWorkflowProgress(['*Waiting for 1 dynamic workflow to finish']);
  assert.equal(r.waitingDynamic, 1);
});

test('parseWorkflowProgress: 待ち行が無いバッファは waitingDynamic:0', () => {
  const r = parseWorkflowProgress(['⚙ workflow w', '  ✓ a']);
  assert.equal(r.waitingDynamic, 0);
});

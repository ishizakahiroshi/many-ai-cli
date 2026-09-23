import assert from 'node:assert/strict';
import test from 'node:test';
import { parseWorkflowProgress } from './workflow-progress.js';
import {
  chooseWorkflowSource,
  extrapolatedWorkflowElapsedSec,
  getHubWorkflowEntry,
  getWorkflowLedger,
  recordWorkflowDoneLabels,
  removeWorkflowStore,
  resetWorkflowStoreForTest,
  setHubWorkflowProgress,
  WORKFLOW_LEDGER_LIMIT,
  WORKFLOW_WS_FRESH_MS,
} from './workflow-store.js';
import type { WorkflowProgress as HubWorkflowProgress } from '../types/proto.js';

// 背景サマリーと Waiting 行は 2026-08-04 の実機ログで構造だけを較正済み。
// 前景ツリーと /workflows ビューは本文・固有名詞を持ち込まない合成 fixture のまま。

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
    '◯ synthetic-review 1/5 agents done · 4m 21s · ↓ 501.0k',
  ]);
  assert.equal(r.detected, true);
  assert.equal(r.running, true);
  assert.equal(r.live, true); // done<total の生サマリー＝凍結 settle 禁止
  assert.equal(r.totalCount, 5);
  assert.equal(r.doneCount, 1);
  assert.equal(r.runningCount, 4);
  assert.equal(r.percent, 20);
});

test('parseWorkflowProgress: サマリーは経過時間込みで frameSig が変わる（凍結誤判定防止）', () => {
  const a = parseWorkflowProgress(['◯ synthetic 1/5 agents done · 4m 21s']);
  const b = parseWorkflowProgress(['◯ synthetic 1/5 agents done · 4m 22s']); // 経過時間だけ進む
  assert.equal(a.runningCount, b.runningCount); // 件数は同じでも
  assert.notEqual(a.frameSig, b.frameSig);      // frameSig は変わる→凍結しない
});

test('parseWorkflowProgress: N/N agents done は完了（live:false）', () => {
  const r = parseWorkflowProgress(['✓ synthetic 5/5 agents done · 7m 34s']);
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

test('parseWorkflowProgress: Tip 文 + 入力欄下のエージェント一覧は誤検出しない（合成データ・実際の行頭記号）', () => {
  // Workflow を使っていないセッションでも、案内文 Tip の "workflow" の語と、
  // 入力欄の下に常時出るエージェント一覧（● main / ◯ …）が組み合わさると
  // 誤検出していた（plan_subagent-tree-popup.md C1）。実ログでは Tip 行の行頭記号は
  // `⎿`（U+23BF）+ U+00A0（実測 866/978 件がこの形）で、行頭記号の除去がこれを
  // 含んでいないと `^tip:` 除外が一致しない（敵対レビュー指摘 R8）。
  const lines = [
    'earlier scrollback line, unrelated to workflows',
    '  ⎿  Tip: Dynamic workflows let Claude coordinate several agents at once — ask Claude to use a workflow directly.',
    '',
    '────────────────────────────────────────',
    '❯ ',
    '⏵⏵ bypass permissions on',
    '● main',
    '◯ general-purpose  Searching …  9m 02s · ↓ 42.5k tokens',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, false);
  assert.equal(r.running, false);
});

test('parseWorkflowProgress: 区切り線が無くても Tip 除外だけで誤検出しない（R8）', () => {
  // 区切り線（罫線・❯ プロンプト）を一切含めず、Tip 行の除外（TREE_PREFIX_RE の
  // `⎿`+U+00A0 対応）だけで誤検出が防げていることを守る。区切り線頼みだと、
  // 区切りが無い/届いていないフレームでは再発する（敵対レビュー指摘 R8）。
  const lines = [
    'earlier scrollback line, unrelated content',
    '  ⎿  Tip: Dynamic workflows let Claude coordinate several agents at once — ask Claude to use a workflow directly.',
    '',
    '⏵⏵ bypass permissions on',
    '● main',
    '◯ general-purpose  Searching …  9m 02s · ↓ 42.5k tokens',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, false);
  assert.equal(r.running, false);
});

test('parseWorkflowProgress: Workflow ブロック内の枠線は入力欄の区切りと誤認せず打ち切らない（R9）', () => {
  // 区切り線の条件は「行全体が横線文字（─━═）と空白だけ」に狭めてある。
  // `╭─── Review ───╮` のように角（╭╮）や文字（Review）を含む行は区切りにしない。
  const lines = [
    'unrelated terminal output',
    '⚙ workflow with-border',
    '╭─── Review ───╮',
    '  ✓ review:bugs',
    '  ⠋ review:perf',
    '╰───────────────╯',
  ];
  const r = parseWorkflowProgress(lines);
  assert.equal(r.detected, true);
  assert.equal(r.totalCount, 2);
  assert.equal(r.doneCount, 1);
  assert.equal(r.runningCount, 1);
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

function hubProgress(patch: Partial<HubWorkflowProgress> = {}): HubWorkflowProgress {
  return {
    detected: true,
    source: 'vt-summary',
    name: 'synthetic',
    done: 1,
    total: 3,
    running: 2,
    failed: 0,
    pending: 0,
    waiting_dynamic: 0,
    percent: 33,
    elapsed_sec: 10,
    phases: [],
    settled: false,
    ...patch,
  };
}

test('workflow store: fresh/settled Hub data outranks local fallback', () => {
  resetWorkflowStoreForTest();
  const at = 1_000_000;
  const fresh = setHubWorkflowProgress(1, hubProgress(), at);
  assert.equal(chooseWorkflowSource(fresh, true, at + WORKFLOW_WS_FRESH_MS), 'hub');
  assert.equal(chooseWorkflowSource(fresh, true, at + WORKFLOW_WS_FRESH_MS + 1), 'local');
  assert.equal(extrapolatedWorkflowElapsedSec(fresh, at + 5_900), 15);

  const settled = setHubWorkflowProgress(1, hubProgress({ settled: true, settled_by: 'journal' }), at);
  assert.equal(chooseWorkflowSource(settled, true, at + WORKFLOW_WS_FRESH_MS * 10), 'hub');
  assert.equal(extrapolatedWorkflowElapsedSec(settled, at + 60_000), 10);
});

test('workflow store: completion ledger caps labels and folds hidden counts', () => {
  resetWorkflowStoreForTest();
  const agents = Array.from({ length: WORKFLOW_LEDGER_LIMIT + 5 }, (_, i) => ({
    label: `agent-${String(i).padStart(3, '0')}`,
    state: 'done',
  }));
  recordWorkflowDoneLabels(2, [{ title: 'synthetic', agents }]);
  const ledger = getWorkflowLedger(2, WORKFLOW_LEDGER_LIMIT + 20);
  assert.equal(ledger.labels.length, WORKFLOW_LEDGER_LIMIT);
  assert.equal(ledger.labels[0], 'agent-005');
  assert.equal(ledger.otherCount, 20);
});

test('workflow store: repeated snapshots do not inflate the hidden count', () => {
  resetWorkflowStoreForTest();
  const agents = Array.from({ length: WORKFLOW_LEDGER_LIMIT + 5 }, (_, i) => ({
    label: `agent-${String(i).padStart(3, '0')}`,
    state: 'done',
  }));
  // 上限超過スナップショットが heartbeat で繰り返し届いても「他 N 件」は
  // 実際の隠れ件数（5）のまま増えないこと（退避カウンタ方式の再発防止）。
  for (let i = 0; i < 10; i++) {
    recordWorkflowDoneLabels(4, [{ title: 'synthetic', agents }]);
  }
  const ledger = getWorkflowLedger(4, WORKFLOW_LEDGER_LIMIT + 5);
  assert.equal(ledger.labels.length, WORKFLOW_LEDGER_LIMIT);
  assert.equal(ledger.otherCount, 5);
});

test('workflow store: session removal clears Hub snapshot and ledger', () => {
  resetWorkflowStoreForTest();
  setHubWorkflowProgress(3, hubProgress({
    phases: [{ title: 'synthetic', agents: [{ label: 'done-agent', state: 'done' }] }],
  }), 10);
  assert.ok(getHubWorkflowEntry(3));
  assert.equal(getWorkflowLedger(3, 1).labels.length, 1);
  removeWorkflowStore(3);
  assert.equal(getHubWorkflowEntry(3), null);
  assert.deepEqual(getWorkflowLedger(3, 0), { labels: [], otherCount: 0 });
});

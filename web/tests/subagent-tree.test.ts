import { describe, expect, test } from 'bun:test';
import { buildSubagentTreeRows } from '../src/app/subagent-tree.ts';
import {
  getSubagentTreeEntry,
  removeSubagentTree,
  removeWorkflowStore,
  setSubagentTree,
} from '../src/app/workflow-store.ts';
import type { SubagentNode, SubagentTree } from '../src/types/proto.ts';

// 子 plan: docs/local/plan_subagent-tree-popup_c5_web-popup.md 内部 C1。
//
// buildSubagentTreeRows は DOM/i18n に依存しない純関数（workflow-progress.ts と
// 同じ分け方）。ここで固定したいのは、深さ優先の並び順・経過時間の算出・
// 親が見つからないノードの除外・空の木を受けたときのストアのクリアの 4 つ。

function node(overrides: Partial<SubagentNode>): SubagentNode {
  return {
    id: 'x',
    depth: 1,
    state: 'running',
    ...overrides,
  } as SubagentNode;
}

describe('buildSubagentTreeRows', () => {
  test('子 A・孫 A-1・子 B が深さ優先（子 A, 孫 A-1, 子 B）で並び、深さが 1・2・1 になる', () => {
    const tree: SubagentTree = {
      nodes: [
        // 入力の並びをわざと崩す（開始時刻順ではない）。
        node({ id: 'b', parent_id: '', started_at: 200, state: 'done', finished_at: 300 }),
        node({ id: 'a1', parent_id: 'a', started_at: 150, state: 'running' }),
        node({ id: 'a', parent_id: '', started_at: 100, state: 'running' }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.map(r => r.id)).toEqual(['a', 'a1', 'b']);
    expect(rows.map(r => r.depth)).toEqual([1, 2, 1]);
  });

  test('兄弟は開始時刻の古い順に並ぶ', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'later', parent_id: '', started_at: 500, state: 'running' }),
        node({ id: 'earlier', parent_id: '', started_at: 100, state: 'running' }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.map(r => r.id)).toEqual(['earlier', 'later']);
  });

  test('経過時間と「最後の動き」は与えた現在時刻から計算され、完了ノードには最後の動きが出ない', () => {
    const tree: SubagentTree = {
      nodes: [
        node({
          id: 'running-node',
          parent_id: '',
          state: 'running',
          started_at: 1_000_000,
          last_activity_at: 1_005_000,
        }),
        node({
          id: 'done-node',
          parent_id: '',
          state: 'done',
          started_at: 2_000_000,
          finished_at: 2_020_000,
        }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1_010_000);
    const running = rows.find(r => r.id === 'running-node')!;
    const done = rows.find(r => r.id === 'done-node')!;

    expect(running.elapsedSec).toBe(10);
    expect(running.lastActivitySec).toBe(5);

    expect(done.elapsedSec).toBe(20);
    expect(done.lastActivitySec).toBeUndefined();
  });

  test('LastToolName が空のノードには今のツールの行が出ない。あるノードには name/summary が出る', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'no-tool', parent_id: '', state: 'running', started_at: 0 }),
        node({
          id: 'with-tool',
          parent_id: '',
          state: 'running',
          started_at: 0,
          last_tool_name: 'Read',
          last_tool_summary: 'file.go',
        }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.find(r => r.id === 'no-tool')!.tool).toBeUndefined();
    expect(rows.find(r => r.id === 'with-tool')!.tool).toEqual({ name: 'Read', summary: 'file.go' });
  });

  test('補足は agent_type と model を · でつなぐ。無い値は落ちる', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'both', parent_id: '', agent_type: 'general-purpose', model: 'sonnet' }),
        node({ id: 'type-only', parent_id: '', agent_type: 'general-purpose' }),
        node({ id: 'neither', parent_id: '' }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.find(r => r.id === 'both')!.detail).toBe('general-purpose · sonnet');
    expect(rows.find(r => r.id === 'type-only')!.detail).toBe('general-purpose');
    expect(rows.find(r => r.id === 'neither')!.detail).toBeUndefined();
  });

  test('名前は label、無ければ agent_type', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'labelled', parent_id: '', label: '設計レビュー', agent_type: 'general-purpose' }),
        node({ id: 'unlabelled', parent_id: '', agent_type: 'general-purpose' }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.find(r => r.id === 'labelled')!.name).toBe('設計レビュー');
    expect(rows.find(r => r.id === 'unlabelled')!.name).toBe('general-purpose');
  });

  test('親が見つからないノードは出さない（孫も連鎖して落ちる）', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'orphan', parent_id: 'ghost-parent', state: 'running' }),
        node({ id: 'orphan-child', parent_id: 'orphan', state: 'running' }),
        node({ id: 'ok', parent_id: '', state: 'running' }),
      ],
    };
    const { rows } = buildSubagentTreeRows(tree, 1000);
    expect(rows.map(r => r.id)).toEqual(['ok']);
  });

  test('集計は走行中・完了・失敗の件数と omitted の素通し', () => {
    const tree: SubagentTree = {
      nodes: [
        node({ id: 'r', parent_id: '', state: 'running' }),
        node({ id: 'd', parent_id: '', state: 'done' }),
        node({ id: 'f', parent_id: '', state: 'failed' }),
      ],
      omitted: 3,
    };
    const { summary } = buildSubagentTreeRows(tree, 1000);
    expect(summary).toEqual({ running: 1, done: 1, failed: 1, omitted: 3 });
  });

  test('空の木、または未指定は空行を返す', () => {
    expect(buildSubagentTreeRows({ nodes: [] }, 1000).rows).toEqual([]);
    expect(buildSubagentTreeRows(null, 1000).rows).toEqual([]);
    expect(buildSubagentTreeRows(undefined, 1000).rows).toEqual([]);
  });
});

describe('subagent tree store（workflow-store.ts）', () => {
  test('木を保持し、getSubagentTreeEntry で読み出せる', () => {
    const sessionId = 9001;
    const tree: SubagentTree = { nodes: [node({ id: 'a', parent_id: '' })] };
    setSubagentTree(sessionId, tree, 12345);
    const entry = getSubagentTreeEntry(sessionId);
    expect(entry).not.toBeNull();
    expect(entry!.receivedAt).toBe(12345);
    expect(entry!.tree.nodes!.map(n => n.id)).toEqual(['a']);
  });

  test('保持後に元のオブジェクトを書き換えても保持分は変わらない（防御的コピー）', () => {
    const sessionId = 9002;
    const nodes = [node({ id: 'a', parent_id: '' })];
    const tree: SubagentTree = { nodes };
    setSubagentTree(sessionId, tree);
    nodes.push(node({ id: 'b', parent_id: '' }));
    const entry = getSubagentTreeEntry(sessionId);
    expect(entry!.tree.nodes!.map(n => n.id)).toEqual(['a']);
  });

  test('空の木（nodes: []）を渡すと保持分が消える', () => {
    const sessionId = 9003;
    setSubagentTree(sessionId, { nodes: [node({ id: 'a', parent_id: '' })] });
    expect(getSubagentTreeEntry(sessionId)).not.toBeNull();
    setSubagentTree(sessionId, { nodes: [] });
    expect(getSubagentTreeEntry(sessionId)).toBeNull();
  });

  test('removeSubagentTree で明示的に消せる', () => {
    const sessionId = 9004;
    setSubagentTree(sessionId, { nodes: [node({ id: 'a', parent_id: '' })] });
    removeSubagentTree(sessionId);
    expect(getSubagentTreeEntry(sessionId)).toBeNull();
  });

  test('removeWorkflowStore はサブエージェントの木も消す（セッション終了時の共通の片付け先）', () => {
    const sessionId = 9005;
    setSubagentTree(sessionId, { nodes: [node({ id: 'a', parent_id: '' })] });
    removeWorkflowStore(sessionId);
    expect(getSubagentTreeEntry(sessionId)).toBeNull();
  });
});

// subagent-tree.ts — サブエージェントの木（Hub からの SubagentTree）を、
// ポップアップ描画用の行の配列へ組み立てる「純粋パーサ」。
//
// workflow-progress.ts と同じ分け方: DOM も i18n も触らない純関数だけを置く。
// 描画・i18n・CSS は C2（web/src/app/workflow-modal.ts ほか）の担当（親 plan:
// docs/local/plan_subagent-tree-popup.md / 子 plan:
// docs/local/plan_subagent-tree-popup_c5_web-popup.md の C1/C2）。
//
// SubagentNode（web/src/types/proto.ts）と同じ規律: プロンプト本文・ツール結果・
// 子の返答を保持するフィールドは持たない。ここで組み立てるのは、そのミラーの値を
// 並べ替えて集計するだけ。

import type { SubagentNode, SubagentTree } from '../types/proto.js';

/** SubagentNode.state の既知の値。未知の値は 'unknown' に丸める。 */
export type SubagentRowState = 'running' | 'done' | 'failed' | 'unknown';

const KNOWN_STATES: SubagentRowState[] = ['running', 'done', 'failed', 'unknown'];

function normalizeState(raw: string | undefined): SubagentRowState {
  return (KNOWN_STATES as string[]).includes(String(raw || '')) ? (raw as SubagentRowState) : 'unknown';
}

/** 走行中の行にだけ付く「今のツール」。i18n の組み立ては呼び出し側（C2）が行う。 */
export interface SubagentToolLine {
  name: string;
  summary?: string;
}

export interface SubagentTreeRow {
  id: string;
  /** 1 = 最上位の子, 2 = 孫, ...（親子チェーンをたどって算出。node.depth の値はそのまま信用しない）。 */
  depth: number;
  state: SubagentRowState;
  /** label。空なら agent_type。どちらも無ければ空文字。 */
  name: string;
  /** agent_type と model を ' · ' でつないだもの。両方無ければ未設定。 */
  detail?: string;
  /** 走行中: now - started_at。完了（finished_at あり）: finished_at - started_at。 */
  elapsedSec?: number;
  /** 走行中だけ: now - last_activity_at。 */
  lastActivitySec?: number;
  /** 走行中で last_tool_name があるときだけ。 */
  tool?: SubagentToolLine;
}

export interface SubagentTreeSummary {
  running: number;
  done: number;
  failed: number;
  /** SubagentTree.omitted の素通し。 */
  omitted: number;
}

export interface SubagentTreeView {
  rows: SubagentTreeRow[];
  summary: SubagentTreeSummary;
}

function secondsBetween(fromMs: number | undefined, toMs: number): number | undefined {
  if (!Number.isFinite(fromMs as number)) return undefined;
  const sec = Math.round((toMs - (fromMs as number)) / 1000);
  return sec > 0 ? sec : 0;
}

function buildDetail(node: SubagentNode): string | undefined {
  const parts = [node.agent_type, node.model]
    .map(v => String(v || '').trim())
    .filter(Boolean);
  return parts.length ? parts.join(' · ') : undefined;
}

function buildRow(node: SubagentNode, depth: number, now: number): SubagentTreeRow {
  const state = normalizeState(node.state);
  const running = state === 'running';
  const row: SubagentTreeRow = {
    id: node.id,
    depth,
    state,
    name: String(node.label || node.agent_type || '').trim(),
  };
  const detail = buildDetail(node);
  if (detail) row.detail = detail;

  if (running) {
    row.elapsedSec = secondsBetween(node.started_at, now);
    row.lastActivitySec = secondsBetween(node.last_activity_at, now);
  } else if (node.finished_at !== undefined && node.finished_at !== null) {
    row.elapsedSec = secondsBetween(node.started_at, node.finished_at);
  }

  if (running && node.last_tool_name) {
    const tool: SubagentToolLine = { name: node.last_tool_name };
    if (node.last_tool_summary) tool.summary = node.last_tool_summary;
    row.tool = tool;
  }

  return row;
}

/**
 * SubagentTree を、ポップアップに描く行の配列と集計へ組み立てる。
 *
 * 並び順は親の直後にその子を置く深さ優先（兄弟は開始時刻の古い順）。
 * 親が見つからないノード（parent_id が非空で、対応する id がツリー内に無い）は
 * 出さない（Hub 側で落としている前提の二重防御・親 plan 方針5）。同じ理由で、
 * 出力対象の親をたどれない孫以降のノードも、たどり着けない時点で自然に落ちる。
 */
export function buildSubagentTreeRows(tree: SubagentTree | null | undefined, now: number): SubagentTreeView {
  const summary: SubagentTreeSummary = {
    running: 0,
    done: 0,
    failed: 0,
    omitted: Math.max(0, Number(tree?.omitted || 0)),
  };
  const nodes: SubagentNode[] = Array.isArray(tree?.nodes) ? (tree!.nodes as SubagentNode[]) : [];
  if (nodes.length === 0) return { rows: [], summary };

  const byId = new Map<string, SubagentNode>();
  for (const n of nodes) {
    if (!n || !n.id) continue;
    byId.set(n.id, n);
  }

  // parentKey '' はツリーの最上位バケット。parent_id が非空なのに対応する id が
  // 無いノードは、どのバケットにも入れない＝出力から落ちる。
  const childrenByParent = new Map<string, SubagentNode[]>();
  for (const n of nodes) {
    if (!n || !n.id) continue;
    const pid = n.parent_id || '';
    if (pid !== '' && !byId.has(pid)) continue;
    if (!childrenByParent.has(pid)) childrenByParent.set(pid, []);
    childrenByParent.get(pid)!.push(n);
  }
  for (const list of childrenByParent.values()) {
    list.sort((a, b) => Number(a.started_at || 0) - Number(b.started_at || 0));
  }

  const rows: SubagentTreeRow[] = [];
  const visited = new Set<string>();

  function walk(parentKey: string, depth: number): void {
    const children = childrenByParent.get(parentKey) || [];
    for (const node of children) {
      // 壊れた入力（同じ id が親子で循環参照）で無限再帰しないための保険。
      if (visited.has(node.id)) continue;
      visited.add(node.id);

      rows.push(buildRow(node, depth, now));
      const state = normalizeState(node.state);
      if (state === 'running') summary.running++;
      else if (state === 'done') summary.done++;
      else if (state === 'failed') summary.failed++;

      walk(node.id, depth + 1);
    }
  }
  walk('', 1);

  return { rows, summary };
}

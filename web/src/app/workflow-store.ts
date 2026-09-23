// workflow-store.ts — Hub workflow snapshots and the in-memory completion ledger.
//
// This module is deliberately DOM-free so source-priority and ledger behavior
// can be covered by the existing Node fixture test suite.

import type { SubagentTree, WfPhase, WorkflowProgress as HubWorkflowProgress } from '../types/proto.js';

export const WORKFLOW_WS_FRESH_MS = 30_000;
export const WORKFLOW_LEDGER_LIMIT = 200;

export interface HubWorkflowEntry {
  progress: HubWorkflowProgress;
  receivedAt: number;
}

interface WorkflowLedgerState {
  labels: string[];
  // seen はセッション寿命の重複排除集合。退避（200 件窓落ち）で削除しない —
  // 削除すると同一ラベルの再追加→再退避のたびに「他 N 件」が水増しされる
  // （敵対レビュー 2026-08-05: 真値 5 のところ 1850 と表示される実証あり）。
  seen: Set<string>;
}

export interface WorkflowLedgerView {
  labels: string[];
  otherCount: number;
}

// サブエージェントの木（子 plan: plan_subagent-tree-popup_c5_web-popup.md 内部
// C1）。Workflow の進捗とは別フィールドで保持する（親 plan 方針6「既存の
// Workflow 表示と権威を混ぜない」）。ディスク非永続・セッションが生きている間
// だけ保持なのは hubEntries と同じ方針。
export interface SubagentTreeEntry {
  tree: SubagentTree;
  receivedAt: number;
}

const hubEntries = new Map<number, HubWorkflowEntry>();
const ledgers = new Map<number, WorkflowLedgerState>();
const subagentTrees = new Map<number, SubagentTreeEntry>();

function cloneHubProgress(progress: HubWorkflowProgress): HubWorkflowProgress {
  return {
    ...progress,
    phases: (progress.phases || []).map(phase => ({
      ...phase,
      agents: (phase.agents || []).map(agent => ({ ...agent })),
    })),
  };
}

export function setHubWorkflowProgress(
  sessionId: number,
  progress: HubWorkflowProgress,
  receivedAt = Date.now(),
): HubWorkflowEntry {
  const entry = { progress: cloneHubProgress(progress), receivedAt };
  hubEntries.set(sessionId, entry);
  recordWorkflowDoneLabels(sessionId, entry.progress.phases || []);
  return entry;
}

export function getHubWorkflowEntry(sessionId: number): HubWorkflowEntry | null {
  return hubEntries.get(sessionId) || null;
}

export function isHubWorkflowAuthoritative(
  entry: HubWorkflowEntry | null | undefined,
  now = Date.now(),
): boolean {
  if (!entry) return false;
  // A Hub settle is conclusive and remains authoritative after the heartbeat
  // stream stops. Only an unsettled snapshot is subject to freshness fallback.
  return entry.progress.settled || now - entry.receivedAt <= WORKFLOW_WS_FRESH_MS;
}

export function chooseWorkflowSource(
  entry: HubWorkflowEntry | null | undefined,
  hasLocalFallback: boolean,
  now = Date.now(),
): 'hub' | 'local' | 'none' {
  if (isHubWorkflowAuthoritative(entry, now)) return 'hub';
  return hasLocalFallback ? 'local' : 'none';
}

export function extrapolatedWorkflowElapsedSec(
  entry: HubWorkflowEntry,
  now = Date.now(),
): number {
  const base = Math.max(0, Number(entry.progress.elapsed_sec || 0));
  if (entry.progress.settled) return base;
  return base + Math.max(0, Math.floor((now - entry.receivedAt) / 1000));
}

export function recordWorkflowDoneLabels(sessionId: number, phases: WfPhase[]): void {
  let ledger = ledgers.get(sessionId);
  if (!ledger) {
    ledger = { labels: [], seen: new Set<string>() };
    ledgers.set(sessionId, ledger);
  }
  for (const phase of phases || []) {
    for (const agent of phase.agents || []) {
      if (agent.state !== 'done') continue;
      const label = String(agent.label || '').replace(/\s+/g, ' ').trim();
      if (!label || ledger.seen.has(label)) continue;
      ledger.seen.add(label);
      ledger.labels.push(label);
      while (ledger.labels.length > WORKFLOW_LEDGER_LIMIT) {
        ledger.labels.shift();
      }
    }
  }
}

export function getWorkflowLedger(sessionId: number, reportedDone = 0): WorkflowLedgerView {
  const ledger = ledgers.get(sessionId);
  if (!ledger) {
    return { labels: [], otherCount: Math.max(0, reportedDone) };
  }
  // "other" covers both labels evicted by the 200-row cap and journal-only
  // completions for which no label was ever visible in the VT tree. Both are
  // recomputed per read; a cumulative counter would double-count re-observed
  // labels and only ever grow.
  const hiddenSeen = Math.max(0, ledger.seen.size - ledger.labels.length);
  const otherCount = Math.max(hiddenSeen, Math.max(0, reportedDone - ledger.labels.length));
  return { labels: ledger.labels.slice(), otherCount };
}

function cloneSubagentTree(tree: SubagentTree): SubagentTree {
  return {
    ...tree,
    nodes: (tree.nodes || []).map(n => ({ ...n })),
  };
}

/**
 * 受け取った木を保持する。ノードが 0 件（Hub が「子が居なくなった」を伝える
 * 空の木・internal/hub/subagent_tree.go の subagentTreeForBroadcastLocked）
 * のときは保持分を消す（子 plan C1 完了条件「空の木を受け取ると、その
 * セッションの保持分が消える」）。
 */
export function setSubagentTree(
  sessionId: number,
  tree: SubagentTree,
  receivedAt = Date.now(),
): void {
  if (!tree || !Array.isArray(tree.nodes) || tree.nodes.length === 0) {
    subagentTrees.delete(sessionId);
    return;
  }
  subagentTrees.set(sessionId, { tree: cloneSubagentTree(tree), receivedAt });
}

export function getSubagentTreeEntry(sessionId: number): SubagentTreeEntry | null {
  return subagentTrees.get(sessionId) || null;
}

export function removeSubagentTree(sessionId: number): void {
  subagentTrees.delete(sessionId);
}

export function removeWorkflowStore(sessionId: number): void {
  hubEntries.delete(sessionId);
  ledgers.delete(sessionId);
  subagentTrees.delete(sessionId);
}

// Test-only reset kept exported because fixture tests run in one Node process.
export function resetWorkflowStoreForTest(): void {
  hubEntries.clear();
  ledgers.clear();
  subagentTrees.clear();
}

package hub

// subagent_source_claude_test.go — 子 plan
// plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C2 の完了条件をすべて合成
// データで確認する。実ファイルの本文は一切使わない（合成のタイムスタンプ・
// ID・ツール名・入力キーのみ）。

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// --- fixture helpers -------------------------------------------------------

func newClaudeSubagentFixtureDirs(t *testing.T) (parentPath, subDir string) {
	t.Helper()
	sessionDir := filepath.Join(t.TempDir(), "sess")
	parentPath = sessionDir + ".jsonl"
	subDir = filepath.Join(sessionDir, "subagents")
	if err := os.MkdirAll(subDir, 0o755); err != nil {
		t.Fatal(err)
	}
	return parentPath, subDir
}

func writeClaudeJSONLLines(t *testing.T, path string, lines ...map[string]any) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = f.Close() }()
	for _, l := range lines {
		data, err := json.Marshal(l)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := f.Write(append(data, '\n')); err != nil {
			t.Fatal(err)
		}
	}
}

func writeClaudeSubagentMetaFile(t *testing.T, subDir, id string, meta claudeSubagentMeta) {
	t.Helper()
	data, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(subDir, "agent-"+id+".meta.json")
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func claudeChildLine(role, ts string, content []map[string]any) map[string]any {
	return map[string]any{
		"type":      role,
		"timestamp": ts,
		"message":   map[string]any{"role": role, "content": content},
	}
}

func claudeToolUseBlock(toolUseID, name string, input map[string]any) map[string]any {
	return map[string]any{"type": "tool_use", "id": toolUseID, "name": name, "input": input}
}

func claudeTextBlock(text string) map[string]any {
	return map[string]any{"type": "text", "text": text}
}

func claudeParentToolUseLine(toolUseID string) map[string]any {
	return map[string]any{
		"type": "assistant",
		"message": map[string]any{"role": "assistant", "content": []any{
			claudeToolUseBlock(toolUseID, "Agent", map[string]any{}),
		}},
	}
}

// claudeParentToolResultLine mirrors the real toolUseResult shape confirmed
// against live records (2026-09-23): isAsync+"async_launched" for a
// backgrounded launch acknowledgement, or absent+"completed"/a "fail..."
// value for a synchronous foreground completion.
func claudeParentToolResultLine(toolUseID, timestamp string, isAsync bool, status string) map[string]any {
	tur := map[string]any{"status": status}
	if isAsync {
		tur["isAsync"] = true
	}
	return map[string]any{
		"type":          "user",
		"timestamp":     timestamp,
		"toolUseResult": tur,
		"message": map[string]any{"role": "user", "content": []any{
			map[string]any{"type": "tool_result", "tool_use_id": toolUseID, "content": "ok"},
		}},
	}
}

// claudeParentNotificationLine mirrors the real
// {"type":"queue-operation","operation":"enqueue","content":"<task-notification>..."}
// shape confirmed against live records.
func claudeParentNotificationLine(toolUseID, taskID, timestamp, status string) map[string]any {
	content := "<task-notification>\n" +
		"<task-id>" + taskID + "</task-id>\n" +
		"<tool-use-id>" + toolUseID + "</tool-use-id>\n" +
		"<status>" + status + "</status>\n" +
		"</task-notification>"
	return map[string]any{
		"type":      "queue-operation",
		"operation": "enqueue",
		"timestamp": timestamp,
		"content":   content,
	}
}

func nodesByID(tree *proto.SubagentTree) map[string]proto.SubagentNode {
	out := map[string]proto.SubagentNode{}
	if tree == nil {
		return out
	}
	for _, n := range tree.Nodes {
		out[n.ID] = n
	}
	return out
}

// --- tests -------------------------------------------------------------

func TestReadClaudeSubagentTreeBuildsShapeAndDropsOrphanGrandchild(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)

	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_c1"),
		claudeParentToolResultLine("toolu_c1", "2026-09-23T10:00:05.000Z", false, "completed"),
		claudeParentToolUseLine("toolu_c2"),
	)

	writeClaudeSubagentMetaFile(t, subDir, "c1", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "child one", ToolUseID: "toolu_c1", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-c1.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	writeClaudeSubagentMetaFile(t, subDir, "c2", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "child two", ToolUseID: "toolu_c2", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-c2.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:01.000Z", []map[string]any{claudeTextBlock("hi")}))

	writeClaudeSubagentMetaFile(t, subDir, "g1", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "grandchild", ToolUseID: "toolu_g1", SpawnDepth: 2, ParentAgentID: "c1",
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-g1.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:02.000Z", []map[string]any{claudeTextBlock("hi")}))

	writeClaudeSubagentMetaFile(t, subDir, "orphan", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "orphan grandchild", ToolUseID: "toolu_orphan", SpawnDepth: 2, ParentAgentID: "does-not-exist",
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-orphan.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:03.000Z", []map[string]any{claudeTextBlock("hi")}))

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	if tree == nil {
		t.Fatal("expected a non-nil tree")
	}
	byID := nodesByID(tree)
	if len(byID) != 3 {
		t.Fatalf("expected 3 nodes (c1, c2, g1; orphan dropped), got %d: %+v", len(byID), byID)
	}
	if _, ok := byID["orphan"]; ok {
		t.Fatal("orphan grandchild (missing parent) must not appear in the tree")
	}
	if g1, ok := byID["g1"]; !ok || g1.ParentID != "c1" || g1.Depth != 2 {
		t.Fatalf("g1 = %+v, want ParentID=c1 Depth=2", g1)
	}
	c1, ok := byID["c1"]
	if !ok || c1.State != "done" {
		t.Fatalf("c1.State = %+v, want done (synchronous toolUseResult completion)", c1)
	}
	c2, ok := byID["c2"]
	if !ok || c2.State != "running" {
		t.Fatalf("c2.State = %+v, want running (no parent-side signal yet, first-seen growth fallback)", c2)
	}
}

func TestReadClaudeSubagentTreeSkipsWorkflowSubagents(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath)

	writeClaudeSubagentMetaFile(t, subDir, "wf1", claudeSubagentMeta{
		AgentType: claudeSubagentWorkflowAgentType, Description: "workflow child", ToolUseID: "toolu_wf1", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-wf1.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	// A "workflows" directory entry directly under subagents/ must be skipped
	// as a directory, never descended into.
	wfDir := filepath.Join(subDir, "workflows", "wf_run1")
	if err := os.MkdirAll(wfDir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeClaudeSubagentMetaFile(t, wfDir, "inner", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "nested workflow agent", ToolUseID: "toolu_inner", SpawnDepth: 1,
	})

	writeClaudeSubagentMetaFile(t, subDir, "normal", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "normal child", ToolUseID: "toolu_normal", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-normal.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	if len(byID) != 1 {
		t.Fatalf("expected only the normal child, got %d nodes: %+v", len(byID), byID)
	}
	if _, ok := byID["normal"]; !ok {
		t.Fatalf("normal child missing from tree: %+v", byID)
	}
}

func TestReadClaudeSubagentTreeLastToolSummaryOnlyAllowedKeys(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath)

	writeClaudeSubagentMetaFile(t, subDir, "allowed", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "allowed key", ToolUseID: "toolu_allowed", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-allowed.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{
			claudeToolUseBlock("t1", "Grep", map[string]any{"other_key": "should not appear", "pattern": "*.go"}),
		}))

	writeClaudeSubagentMetaFile(t, subDir, "disallowed", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "disallowed only", ToolUseID: "toolu_disallowed", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-disallowed.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{
			claudeToolUseBlock("t2", "SomeTool", map[string]any{"other_key": "must not leak into summary"}),
		}))

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	allowed, ok := byID["allowed"]
	if !ok || allowed.LastToolSummary != "*.go" {
		t.Fatalf("allowed.LastToolSummary = %q, want %q", allowed.LastToolSummary, "*.go")
	}
	disallowed, ok := byID["disallowed"]
	if !ok || disallowed.LastToolSummary != "" {
		t.Fatalf("disallowed.LastToolSummary = %q, want empty (no allow-listed key present)", disallowed.LastToolSummary)
	}
	if strings.Contains(disallowed.LastToolSummary, "leak") {
		t.Fatalf("disallowed key value leaked into LastToolSummary: %q", disallowed.LastToolSummary)
	}
}

func TestReadClaudeSubagentTreeSinceFilterKeepsRunningDropsOldDone(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	since := time.Date(2026, 6, 1, 0, 0, 0, 0, time.UTC)

	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_old_done"),
		claudeParentToolResultLine("toolu_old_done", "2026-01-01T00:05:00.000Z", false, "completed"),
	)

	// Started well before `since`, and the parent transcript resolves it as
	// done — must not appear (方針 4: 走行中の子が 0 人になった後の入力で区切る).
	writeClaudeSubagentMetaFile(t, subDir, "old-done", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "old done", ToolUseID: "toolu_old_done", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-old-done.jsonl"),
		claudeChildLine("assistant", "2026-01-01T00:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	// Started before `since` too, but has no parent-side completion signal at
	// all, so the growth fallback marks it running — must still appear.
	writeClaudeSubagentMetaFile(t, subDir, "old-running", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "old but running", ToolUseID: "toolu_old_running", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-old-running.jsonl"),
		claudeChildLine("assistant", "2026-01-01T00:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	tree, _, err := readClaudeSubagentTree(parentPath, since, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	if _, ok := byID["old-done"]; ok {
		t.Fatalf("old-done (finished before since) must be filtered out: %+v", byID)
	}
	running, ok := byID["old-running"]
	if !ok || running.State != "running" {
		t.Fatalf("old-running must stay in the tree as running, got %+v (present=%v)", running, ok)
	}
}

func TestReadClaudeSubagentTreeCapsAtMaxNodesDropsOldestCompletedFirst(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)

	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_a"),
		claudeParentToolResultLine("toolu_a", "2026-09-23T10:00:01.000Z", false, "completed"),
		claudeParentToolUseLine("toolu_b"),
		claudeParentToolResultLine("toolu_b", "2026-09-23T10:00:02.000Z", false, "completed"),
		claudeParentToolUseLine("toolu_c"),
		claudeParentToolResultLine("toolu_c", "2026-09-23T10:00:03.000Z", false, "completed"),
	)
	for _, id := range []string{"a", "b", "c"} {
		writeClaudeSubagentMetaFile(t, subDir, id, claudeSubagentMeta{
			AgentType: "general-purpose", Description: "agent " + id, ToolUseID: "toolu_" + id, SpawnDepth: 1,
		})
		writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-"+id+".jsonl"),
			claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))
	}

	budget := defaultSubagentReadBudget()
	budget.MaxNodes = 2
	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, budget)
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	if tree == nil {
		t.Fatal("expected a non-nil tree")
	}
	if len(tree.Nodes) != 2 {
		t.Fatalf("expected exactly 2 nodes (MaxNodes cap), got %d: %+v", len(tree.Nodes), tree.Nodes)
	}
	if tree.Omitted != 1 {
		t.Fatalf("tree.Omitted = %d, want 1", tree.Omitted)
	}
	byID := nodesByID(tree)
	if _, ok := byID["a"]; ok {
		t.Fatalf("oldest-finished agent 'a' should have been dropped first: %+v", byID)
	}
	if _, ok := byID["b"]; !ok {
		t.Fatalf("agent 'b' should have survived the cap: %+v", byID)
	}
	if _, ok := byID["c"]; !ok {
		t.Fatalf("agent 'c' should have survived the cap: %+v", byID)
	}
}

func TestReadClaudeSubagentTreeAsyncBackgroundCompletionViaTaskNotification(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_bg"),
		// The immediate tool_result for a backgrounded child is a launch
		// acknowledgement only, per the real isAsync/"async_launched" shape
		// confirmed 2026-09-23 — must NOT be read as completion.
		claudeParentToolResultLine("toolu_bg", "2026-09-23T10:00:01.000Z", true, "async_launched"),
		claudeParentNotificationLine("toolu_bg", "shortid123", "2026-09-23T10:05:00.000Z", "failed"),
	)
	writeClaudeSubagentMetaFile(t, subDir, "bg", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "background child", ToolUseID: "toolu_bg", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-bg.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	bg, ok := byID["bg"]
	if !ok {
		t.Fatalf("background child missing from tree: %+v", byID)
	}
	if bg.State != "failed" {
		t.Fatalf("bg.State = %q, want failed (from the task-notification's <status>, not the async_launched tool_result)", bg.State)
	}
}

// TestReadClaudeSubagentTreeRespectsChildReadBudgets is 子 plan C2 の完了条件
// "10MB の合成 jsonl で、1 回の読み取りが先頭 64KB と末尾 128KB を超えない" の直接
// 確認。末尾 128KB の外にしか tool_use が無ければ見つからず、先頭 64KB を超える
// ファイルでは ToolCalls を数え切れない（＝出さない）ことを、実際に 10MB のファイル
// を読ませて確認する（読み取りが無制限なら両方とも「見つかる/数えられる」になる）。
func TestReadClaudeSubagentTreeRespectsChildReadBudgets(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath)

	writeClaudeSubagentMetaFile(t, subDir, "big", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "big child", ToolUseID: "toolu_big", SpawnDepth: 1,
	})

	childPath := filepath.Join(subDir, "agent-big.jsonl")
	f, err := os.Create(childPath)
	if err != nil {
		t.Fatal(err)
	}
	writeOne := func(v map[string]any) {
		data, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := f.Write(append(data, '\n')); err != nil {
			t.Fatal(err)
		}
	}
	// First line: the true start, with a tool_use that sits well outside any
	// tail window once the file grows past 10MB.
	writeOne(claudeChildLine("assistant", "2026-09-23T09:00:00.000Z", []map[string]any{
		claudeToolUseBlock("t-early", "OnlyEarlyTool", map[string]any{"command": "echo early"}),
	}))
	// Pad with plain text-only records (no tool_use) until the file is
	// comfortably past 10MB, so the last 128KB contains no tool_use at all.
	padLine := claudeChildLine("assistant", "2026-09-23T09:00:01.000Z", []map[string]any{
		claudeTextBlock(strings.Repeat("x", 900)),
	})
	padData, err := json.Marshal(padLine)
	if err != nil {
		t.Fatal(err)
	}
	padBytes := int64(len(padData) + 1)
	var written int64
	const targetSize = 10 * 1024 * 1024
	for written < targetSize {
		writeOne(padLine)
		written += padBytes
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	info, err := os.Stat(childPath)
	if err != nil {
		t.Fatal(err)
	}
	if info.Size() < targetSize {
		t.Fatalf("fixture file too small: %d bytes", info.Size())
	}

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	big, ok := byID["big"]
	if !ok {
		t.Fatalf("big child missing from tree: %+v", byID)
	}
	if big.StartedAt == 0 {
		t.Fatal("StartedAt must still be resolved from the head window even on a 10MB file")
	}
	if big.LastToolName != "" || big.LastToolSummary != "" {
		t.Fatalf("LastToolName/LastToolSummary = %q/%q, want empty: the only tool_use is outside the 128KB tail window",
			big.LastToolName, big.LastToolSummary)
	}
	if big.ToolCalls != 0 {
		t.Fatalf("ToolCalls = %d, want 0/unset: a 10MB file exceeds the 64KB head budget so the count is not complete", big.ToolCalls)
	}

	// Also exercise the head reader directly for the toolCallsKnown signal,
	// which proto.SubagentNode's omitempty ToolCalls can't distinguish from
	// "known zero" on its own.
	_, _, known := claudeSubagentReadHead(childPath, defaultSubagentReadBudget().HeadBytes, info.Size())
	if known {
		t.Fatal("toolCallsKnown must be false once the child transcript exceeds the head budget")
	}
}

// --- C7: 起動の信号〜完了の信号 state machine --------------------------------
//
// The tests below cover 親 plan
// docs/local/plan_subagent-tree-popup.md の C7 完了条件. They replace the old
// "did the jsonl grow since the last poll" rule (which flickered a still-running
// child to "unknown" the instant it went quiet for one poll) with an explicit
// launch/completion signal state machine, kept sticky in claudeSubagentChildCache
// across polls.

// TestReadClaudeSubagentTreeBackgroundChildStaysRunningAcrossStalledPolls is
// C7 完了条件 "合成データで、裏で起動した子の記録が 30 秒伸びない（poll を 10 回
// 回す）間も running のままで、unknown にならない". The child's own jsonl is
// backdated to 30 minutes ago *before* the very first poll, well outside
// claudeSubagentFreshnessWindow — so a pass here can only be explained by the
// isAsync launch signal itself (真の sticky running), never by "recently
// modified" luck.
func TestReadClaudeSubagentTreeBackgroundChildStaysRunningAcrossStalledPolls(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_stall"),
		claudeParentToolResultLine("toolu_stall", "2026-09-23T10:00:01.000Z", true, "async_launched"),
	)
	writeClaudeSubagentMetaFile(t, subDir, "stall", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "long test", ToolUseID: "toolu_stall", SpawnDepth: 1,
	})
	childPath := filepath.Join(subDir, "agent-stall.jsonl")
	writeClaudeJSONLLines(t, childPath,
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	stale := time.Now().Add(-30 * time.Minute)
	if err := os.Chtimes(childPath, stale, stale); err != nil {
		t.Fatal(err)
	}

	var prior any
	for i := 0; i < 10; i++ {
		tree, next, err := readClaudeSubagentTree(parentPath, time.Time{}, prior, defaultSubagentReadBudget())
		if err != nil {
			t.Fatalf("poll %d: readClaudeSubagentTree error: %v", i, err)
		}
		prior = next
		byID := nodesByID(tree)
		stall, ok := byID["stall"]
		if !ok || stall.State != "running" {
			t.Fatalf("poll %d: stall.State = %+v (present=%v), want running (backgrounded launch signal must win over jsonl staleness, no unknown flicker)", i, stall, ok)
		}
	}
}

// TestReadClaudeSubagentTreeSkipsParentRescanWhenUnchanged is C7 R6 完了条件
// "親の記録が変わっていない poll では、親の記録を開かない（呼び出し回数をテストで
// 数える）". Ten polls over an untouched parent transcript must scan it exactly
// once.
func TestReadClaudeSubagentTreeSkipsParentRescanWhenUnchanged(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_stall2"),
		claudeParentToolResultLine("toolu_stall2", "2026-09-23T10:00:01.000Z", true, "async_launched"),
	)
	writeClaudeSubagentMetaFile(t, subDir, "stall2", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "long test", ToolUseID: "toolu_stall2", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-stall2.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	var calls int
	orig := claudeSubagentScanParentFn
	claudeSubagentScanParentFn = func(path string) map[string]claudeSubagentParentSignal {
		calls++
		return orig(path)
	}
	defer func() { claudeSubagentScanParentFn = orig }()

	var prior any
	for i := 0; i < 10; i++ {
		_, next, err := readClaudeSubagentTree(parentPath, time.Time{}, prior, defaultSubagentReadBudget())
		if err != nil {
			t.Fatalf("poll %d: readClaudeSubagentTree error: %v", i, err)
		}
		prior = next
	}
	if calls != 1 {
		t.Fatalf("claudeSubagentScanParentFn called %d times across 10 polls of an unchanged parent transcript, want 1 (R6)", calls)
	}
}

// TestReadClaudeSubagentTreeNoParentSignalFallsBackToJSONLFreshness is C7
// 完了条件 "起動も完了も一度も見ていない子は、記録の更新が 10 分以内なら running、
// それより古ければ unknown". Neither child has any parent-side evidence at
// all; only their own jsonl mtime differs.
func TestReadClaudeSubagentTreeNoParentSignalFallsBackToJSONLFreshness(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath) // no evidence for either child at all

	writeClaudeSubagentMetaFile(t, subDir, "fresh", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "fresh", ToolUseID: "toolu_fresh", SpawnDepth: 1,
	})
	freshPath := filepath.Join(subDir, "agent-fresh.jsonl")
	writeClaudeJSONLLines(t, freshPath,
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))
	// freshPath's mtime is "now" (just written) — well inside the fallback window.

	writeClaudeSubagentMetaFile(t, subDir, "stale", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "stale", ToolUseID: "toolu_stale", SpawnDepth: 1,
	})
	stalePath := filepath.Join(subDir, "agent-stale.jsonl")
	writeClaudeJSONLLines(t, stalePath,
		claudeChildLine("assistant", "2026-09-23T09:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))
	old := time.Now().Add(-20 * time.Minute)
	if err := os.Chtimes(stalePath, old, old); err != nil {
		t.Fatal(err)
	}

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	byID := nodesByID(tree)
	if fresh, ok := byID["fresh"]; !ok || fresh.State != "running" {
		t.Fatalf("fresh.State = %+v (present=%v), want running (jsonl modified within the fallback window)", fresh, ok)
	}
	if stale, ok := byID["stale"]; !ok || stale.State != "unknown" {
		t.Fatalf("stale.State = %+v (present=%v), want unknown (no parent signal and jsonl older than the fallback window)", stale, ok)
	}
}

// TestReadClaudeSubagentTreeForegroundRunningUntilToolResultArrives is C7
// 完了条件 "表の呼び出しも、tool_result が来るまで running、来たら done". Poll 1
// sees only the Agent tool_use (no result yet); poll 2's parent transcript
// has grown a matching, non-async, "completed" tool_result.
func TestReadClaudeSubagentTreeForegroundRunningUntilToolResultArrives(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath, claudeParentToolUseLine("toolu_fg"))
	writeClaudeSubagentMetaFile(t, subDir, "fg", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "fg", ToolUseID: "toolu_fg", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-fg.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	tree1, next, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("poll 1: readClaudeSubagentTree error: %v", err)
	}
	byID1 := nodesByID(tree1)
	if fg, ok := byID1["fg"]; !ok || fg.State != "running" {
		t.Fatalf("poll 1: fg.State = %+v (present=%v), want running (Agent tool_use seen, no tool_result yet)", fg, ok)
	}

	// The parent transcript grows: the tool_result completing the Agent call
	// arrives.
	writeClaudeJSONLLines(t, parentPath,
		claudeParentToolUseLine("toolu_fg"),
		claudeParentToolResultLine("toolu_fg", "2026-09-23T10:05:00.000Z", false, "completed"),
	)
	tree2, _, err := readClaudeSubagentTree(parentPath, time.Time{}, next, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("poll 2: readClaudeSubagentTree error: %v", err)
	}
	byID2 := nodesByID(tree2)
	fg2, ok := byID2["fg"]
	if !ok || fg2.State != "done" {
		t.Fatalf("poll 2: fg.State = %+v (present=%v), want done (synchronous tool_result completed)", fg2, ok)
	}
	if fg2.FinishedAt == 0 {
		t.Fatal("poll 2: fg.FinishedAt must be set once resolved")
	}
}

// TestReadClaudeSubagentTreeCountsChildReadBytesDirectly is C7 R10 完了条件
// "読み取りバイト数の上限をテストで直接数えている": measures the actual byte
// counts claudeSubagentReadHead/claudeSubagentReadLastTool perform against a
// 10MB child transcript, rather than inferring the budget was honored from
// downstream behavior only.
func TestReadClaudeSubagentTreeCountsChildReadBytesDirectly(t *testing.T) {
	parentPath, subDir := newClaudeSubagentFixtureDirs(t)
	writeClaudeJSONLLines(t, parentPath)

	writeClaudeSubagentMetaFile(t, subDir, "big2", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "big child 2", ToolUseID: "toolu_big2", SpawnDepth: 1,
	})

	childPath := filepath.Join(subDir, "agent-big2.jsonl")
	f, err := os.Create(childPath)
	if err != nil {
		t.Fatal(err)
	}
	writeOne := func(v map[string]any) {
		data, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := f.Write(append(data, '\n')); err != nil {
			t.Fatal(err)
		}
	}
	writeOne(claudeChildLine("assistant", "2026-09-23T09:00:00.000Z", []map[string]any{
		claudeToolUseBlock("t-early2", "OnlyEarlyTool", map[string]any{"command": "echo early"}),
	}))
	padLine := claudeChildLine("assistant", "2026-09-23T09:00:01.000Z", []map[string]any{
		claudeTextBlock(strings.Repeat("x", 900)),
	})
	padData, err := json.Marshal(padLine)
	if err != nil {
		t.Fatal(err)
	}
	padBytes := int64(len(padData) + 1)
	var written int64
	const targetSize = 10 * 1024 * 1024
	for written < targetSize {
		writeOne(padLine)
		written += padBytes
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	budget := defaultSubagentReadBudget()
	var headSamples, tailSamples []int64
	prevObserver := claudeSubagentReadObserver
	claudeSubagentReadObserver = func(kind string, n int64) {
		switch kind {
		case "head":
			headSamples = append(headSamples, n)
		case "tail":
			tailSamples = append(tailSamples, n)
		}
	}
	defer func() { claudeSubagentReadObserver = prevObserver }()

	tree, _, err := readClaudeSubagentTree(parentPath, time.Time{}, nil, budget)
	if err != nil {
		t.Fatalf("readClaudeSubagentTree error: %v", err)
	}
	if tree == nil {
		t.Fatal("expected a non-nil tree")
	}
	if len(headSamples) == 0 {
		t.Fatal("expected at least one observed head read")
	}
	if len(tailSamples) == 0 {
		t.Fatal("expected at least one observed tail read")
	}
	for _, n := range headSamples {
		if n > budget.HeadBytes {
			t.Fatalf("observed head read of %d bytes exceeds HeadBytes budget %d", n, budget.HeadBytes)
		}
	}
	for _, n := range tailSamples {
		if n > budget.TailBytes {
			t.Fatalf("observed tail read of %d bytes exceeds TailBytes budget %d", n, budget.TailBytes)
		}
	}
}

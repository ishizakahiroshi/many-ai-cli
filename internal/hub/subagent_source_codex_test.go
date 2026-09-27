package hub

// subagent_source_codex_test.go — 親 plan docs/local/plan_subagent-tree-popup.md
// C3 の完了条件をすべて合成データで確認する。実ファイルの本文は一切使わない
// （合成のタイムスタンプ・ID・ツール名・入力のみ）。

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

// newCodexSubagentFixtureRoot creates a synthetic CODEX_HOME/sessions/ tree
// with a single day directory dated dayDate (local, truncated to midnight),
// returning the sessions root and that day's directory.
func newCodexSubagentFixtureRoot(t *testing.T, dayDate time.Time) (sessionsRoot, dayDir string) {
	t.Helper()
	root := t.TempDir()
	sessionsRoot = filepath.Join(root, "sessions")
	dayDir = filepath.Join(sessionsRoot,
		dayDate.Format("2006"), dayDate.Format("01"), dayDate.Format("02"))
	if err := os.MkdirAll(dayDir, 0o755); err != nil {
		t.Fatal(err)
	}
	return sessionsRoot, dayDir
}

func writeCodexRolloutLines(t *testing.T, path string, lines ...map[string]any) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
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

// codexRootSessionMetaLine is a non-subagent (top-level) rollout's first
// line: it carries payload.id/timestamp but no source.subagent at all.
func codexRootSessionMetaLine(id, timestamp string) map[string]any {
	return map[string]any{
		"type":    "session_meta",
		"payload": map[string]any{"id": id, "timestamp": timestamp},
	}
}

// codexChildSessionMetaLine is a spawned child's first line: adds
// payload.source.subagent.thread_spawn (agent_nickname/agent_role/
// parent_thread_id/depth), confirmed 2026-09-23 to be the exact real shape.
func codexChildSessionMetaLine(id, timestamp, parentThreadID, nickname, role string, depth int) map[string]any {
	return map[string]any{
		"type": "session_meta",
		"payload": map[string]any{
			"id":        id,
			"timestamp": timestamp,
			"source": map[string]any{
				"subagent": map[string]any{
					"thread_spawn": map[string]any{
						"agent_nickname":   nickname,
						"agent_role":       role,
						"parent_thread_id": parentThreadID,
						"depth":            depth,
					},
				},
			},
		},
	}
}

// codexSubAgentActivityLine mirrors the real
// {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"SubAgentActivity","kind":...,"agent_thread_id":...},"completed_at_ms":...}}
// shape confirmed against live parent rollouts (2026-09-23).
func codexSubAgentActivityLine(agentThreadID, kind string, completedAtMs int64) map[string]any {
	return map[string]any{
		"type": "event_msg",
		"payload": map[string]any{
			"type":      "item_completed",
			"thread_id": "parent-thread",
			"turn_id":   "turn-1",
			"item": map[string]any{
				"id":              "item-1",
				"type":            "SubAgentActivity",
				"kind":            kind,
				"agent_thread_id": agentThreadID,
				"agent_path":      "/",
			},
			"completed_at_ms": completedAtMs,
		},
	}
}

// codexCustomToolCallLine mirrors the real
// {"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"<plain string>"}}
// shape (payload.input is a bare JSON string, not an object — confirmed
// 2,215/2,215 real "exec" samples).
func codexCustomToolCallLine(name, input string) map[string]any {
	return map[string]any{
		"type": "response_item",
		"payload": map[string]any{
			"type":    "custom_tool_call",
			"name":    name,
			"input":   input,
			"call_id": "call-1",
			"id":      "item-x",
			"status":  "completed",
		},
	}
}

// codexFunctionCallLine mirrors the real
// {"type":"response_item","payload":{"type":"function_call","name":...,"arguments":"<json-encoded object string>"}}
// shape.
func codexFunctionCallLine(name string, args map[string]any) map[string]any {
	argsJSON, err := json.Marshal(args)
	if err != nil {
		panic(err)
	}
	return map[string]any{
		"type": "response_item",
		"payload": map[string]any{
			"type":      "function_call",
			"name":      name,
			"arguments": string(argsJSON),
			"call_id":   "call-2",
			"id":        "item-y",
		},
	}
}

func codexNodesByID(tree *proto.SubagentTree) map[string]proto.SubagentNode {
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

func TestReadCodexSubagentTreeBuildsShapeAndDropsOrphanGrandchild(t *testing.T) {
	day := time.Now()
	sessionsRoot, dayDir := newCodexSubagentFixtureRoot(t, day)
	_ = sessionsRoot

	ts := day.Format(time.RFC3339)
	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", ts))

	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-c1.jsonl"),
		codexChildSessionMetaLine("c1", ts, "root1", "helper-one", "reviewer", 1))
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-c2.jsonl"),
		codexChildSessionMetaLine("c2", ts, "root1", "helper-two", "coder", 1))
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-g1.jsonl"),
		codexChildSessionMetaLine("g1", ts, "c1", "grandchild", "tester", 2))
	// parent_thread_id names an id that never appears anywhere — must be
	// dropped, not shown as a false top-level node.
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-orphan.jsonl"),
		codexChildSessionMetaLine("orphan", ts, "does-not-exist", "orphan", "role", 2))

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	if tree == nil {
		t.Fatal("expected a non-nil tree")
	}
	byID := codexNodesByID(tree)
	if len(byID) != 3 {
		t.Fatalf("expected 3 nodes (c1, c2, g1; orphan dropped), got %d: %+v", len(byID), byID)
	}
	if _, ok := byID["orphan"]; ok {
		t.Fatal("orphan grandchild (unreachable from root) must not appear in the tree")
	}
	c1, ok := byID["c1"]
	if !ok || c1.ParentID != "" || c1.Depth != 1 || c1.Label != "helper-one" {
		t.Fatalf("c1 = %+v, want ParentID=\"\" Depth=1 Label=helper-one", c1)
	}
	g1, ok := byID["g1"]
	if !ok || g1.ParentID != "c1" || g1.Depth != 2 {
		t.Fatalf("g1 = %+v, want ParentID=c1 Depth=2 (parent_thread_id names a sibling, not the root)", g1)
	}
}

func TestReadCodexSubagentTreeStateTransitionsViaSubAgentActivity(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath,
		codexRootSessionMetaLine("root1", ts),
		codexSubAgentActivityLine("c-done", "started", 0),
		codexSubAgentActivityLine("c-done", "completed", 1758600000000),
		codexSubAgentActivityLine("c-failed", "started", 0),
		codexSubAgentActivityLine("c-failed", "interrupted", 1758600001000),
		codexSubAgentActivityLine("c-running", "started", 0),
	)
	for _, id := range []string{"c-done", "c-failed", "c-running"} {
		writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-"+id+".jsonl"),
			codexChildSessionMetaLine(id, ts, "root1", "nick-"+id, "role", 1))
	}

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	byID := codexNodesByID(tree)
	if done, ok := byID["c-done"]; !ok || done.State != "done" || done.FinishedAt != 1758600000000 {
		t.Fatalf("c-done = %+v, want State=done FinishedAt=1758600000000", done)
	}
	if failed, ok := byID["c-failed"]; !ok || failed.State != "failed" || failed.FinishedAt != 1758600001000 {
		t.Fatalf("c-failed = %+v, want State=failed FinishedAt=1758600001000", failed)
	}
	if running, ok := byID["c-running"]; !ok || running.State != "running" {
		t.Fatalf("c-running = %+v, want State=running (started signal seen, no completed/interrupted signal yet)", running)
	}
}

// TestReadCodexSubagentTreeRunningSurvivesPollsWithoutGrowth is 修正 C8
// （親 plan §C8）の直接確認・指摘 R1 の再現テスト: 「started の後に記録が伸び
// ない poll を10回回しても running のまま」（C8 完了条件）。child の rollout
// を最初の poll の後は一切書き換えない（size/mtime 不変 = 旧コードの Grew は
// 2 poll 目以降 false に落ちる）のに、running が保たれることを確認する。
func TestReadCodexSubagentTreeRunningSurvivesPollsWithoutGrowth(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath,
		codexRootSessionMetaLine("root1", ts),
		codexSubAgentActivityLine("c-running", "started", 0),
	)
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-c-running.jsonl"),
		codexChildSessionMetaLine("c-running", ts, "root1", "nick-c-running", "role", 1))

	var state any
	for poll := 1; poll <= 10; poll++ {
		tree, nextState, err := readCodexSubagentTree(parentPath, time.Time{}, state, defaultSubagentReadBudget())
		if err != nil {
			t.Fatalf("poll %d: readCodexSubagentTree error: %v", poll, err)
		}
		state = nextState
		byID := codexNodesByID(tree)
		running, ok := byID["c-running"]
		if !ok || running.State != "running" {
			t.Fatalf("poll %d: c-running = %+v (present=%v), want State=running even though neither the child's own rollout nor the parent's rollout changed since poll 1", poll, running, ok)
		}
	}
}

// TestReadCodexSubagentTreeRunningSurvivesStartedSignalAgingOutOfParentWindow
// 確認: started の SubAgentActivity レコードが、後続の poll では親の tail scan
// の record 上限（codexSubagentParentReadRecordsMax=500）の外に押し出されても、
// 一度 running と判定した子は running のまま保たれる（読み取り状態に覚えてお
// く。親 plan「修正の C の共通の決まり」）。
func TestReadCodexSubagentTreeRunningSurvivesStartedSignalAgingOutOfParentWindow(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath,
		codexRootSessionMetaLine("root1", ts),
		codexSubAgentActivityLine("c-running", "started", 0),
	)
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-c-running.jsonl"),
		codexChildSessionMetaLine("c-running", ts, "root1", "nick-c-running", "role", 1))

	tree1, state1, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("first poll error: %v", err)
	}
	byID1 := codexNodesByID(tree1)
	if running, ok := byID1["c-running"]; !ok || running.State != "running" {
		t.Fatalf("first poll: c-running = %+v (present=%v), want State=running", running, ok)
	}

	// 501 件の無関係な filler レコードを started の後に足し、
	// codexSubagentParentReadRecordsMax(=500) を超えさせる。これで次の poll の
	// tail scan には started レコードがもう含まれない。
	f, err := os.OpenFile(parentPath, os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 501; i++ {
		filler := map[string]any{
			"type":    "event_msg",
			"payload": map[string]any{"type": "agent_reasoning", "text": "filler"},
		}
		data, err := json.Marshal(filler)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := f.Write(append(data, '\n')); err != nil {
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	tree2, _, err := readCodexSubagentTree(parentPath, time.Time{}, state1, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("second poll error: %v", err)
	}
	byID2 := codexNodesByID(tree2)
	if running, ok := byID2["c-running"]; !ok || running.State != "running" {
		t.Fatalf("second poll: c-running = %+v (present=%v), want State=running even though the started record scrolled outside this poll's bounded tail scan of the parent rollout", running, ok)
	}
}

// TestReadCodexSubagentTreeUnseenSignalFallbackUsesTenMinuteWindow は「起動も
// 完了も一度も見ていない子」の fallback（親 plan「修正の C の共通の決まり」）
// を直接確認する: 子自身の rollout の更新が 10 分以内なら running、それより
// 古ければ unknown。どちらの子も親の rollout に SubAgentActivity は一切無い。
func TestReadCodexSubagentTreeUnseenSignalFallbackUsesTenMinuteWindow(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", ts))

	recentPath := filepath.Join(dayDir, "rollout-recent.jsonl")
	writeCodexRolloutLines(t, recentPath,
		codexChildSessionMetaLine("recent", ts, "root1", "recent", "role", 1))

	stalePath := filepath.Join(dayDir, "rollout-stale.jsonl")
	writeCodexRolloutLines(t, stalePath,
		codexChildSessionMetaLine("stale", ts, "root1", "stale", "role", 1))
	staleTime := time.Now().Add(-20 * time.Minute)
	if err := os.Chtimes(stalePath, staleTime, staleTime); err != nil {
		t.Fatal(err)
	}

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	byID := codexNodesByID(tree)
	if recent, ok := byID["recent"]; !ok || recent.State != "running" {
		t.Fatalf("recent = %+v (present=%v), want State=running (own rollout modified within the last 10 minutes, no signal ever seen)", recent, ok)
	}
	if stale, ok := byID["stale"]; !ok || stale.State != "unknown" {
		t.Fatalf("stale = %+v (present=%v), want State=unknown (own rollout modified 20 minutes ago, no signal ever seen)", stale, ok)
	}
}

func TestReadCodexSubagentTreeLastToolFromCustomToolCallAndFunctionCall(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", ts))

	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-exec.jsonl"),
		codexChildSessionMetaLine("exec-child", ts, "root1", "execer", "role", 1),
		codexCustomToolCallLine("exec", "echo hello world"))

	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-coord.jsonl"),
		codexChildSessionMetaLine("coord-child", ts, "root1", "coordinator", "role", 1),
		codexFunctionCallLine("send_message", map[string]any{"message": "must not leak into summary", "target": "peer"}))

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	byID := codexNodesByID(tree)

	execChild, ok := byID["exec-child"]
	if !ok || execChild.LastToolName != "exec" || execChild.LastToolSummary != "echo hello world" {
		t.Fatalf("exec-child = %+v, want LastToolName=exec LastToolSummary=%q", execChild, "echo hello world")
	}

	coordChild, ok := byID["coord-child"]
	if !ok || coordChild.LastToolName != "send_message" {
		t.Fatalf("coord-child.LastToolName = %q, want send_message", coordChild.LastToolName)
	}
	if coordChild.LastToolSummary != "" {
		t.Fatalf("coord-child.LastToolSummary = %q, want empty (function_call arguments have no allow-listed key)", coordChild.LastToolSummary)
	}
	if strings.Contains(coordChild.LastToolSummary, "leak") {
		t.Fatalf("function_call argument value leaked into LastToolSummary: %q", coordChild.LastToolSummary)
	}
}

func TestReadCodexSubagentTreeSinceFilterKeepsRunningDropsOldDone(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	since := time.Date(2026, 6, 1, 0, 0, 0, 0, time.UTC)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath,
		codexRootSessionMetaLine("root1", day.Format(time.RFC3339)),
		codexSubAgentActivityLine("old-done", "completed", time.Date(2026, 1, 1, 0, 5, 0, 0, time.UTC).UnixMilli()),
	)

	// Started well before `since`, and the parent rollout resolves it as
	// done — must not appear (方針 4).
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-old-done.jsonl"),
		codexChildSessionMetaLine("old-done", "2026-01-01T00:00:00Z", "root1", "old-done", "role", 1))

	// Started before `since` too, but has no parent-side completion signal
	// at all, so the growth fallback marks it running — must still appear.
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-old-running.jsonl"),
		codexChildSessionMetaLine("old-running", "2026-01-01T00:00:00Z", "root1", "old-running", "role", 1))

	tree, _, err := readCodexSubagentTree(parentPath, since, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	byID := codexNodesByID(tree)
	if _, ok := byID["old-done"]; ok {
		t.Fatalf("old-done (finished before since) must be filtered out: %+v", byID)
	}
	running, ok := byID["old-running"]
	if !ok || running.State != "running" {
		t.Fatalf("old-running must stay in the tree as running, got %+v (present=%v)", running, ok)
	}
}

func TestReadCodexSubagentTreeCapsAtMaxNodesDropsOldestCompletedFirst(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath,
		codexRootSessionMetaLine("root1", ts),
		codexSubAgentActivityLine("a", "completed", 1000),
		codexSubAgentActivityLine("b", "completed", 2000),
		codexSubAgentActivityLine("c", "completed", 3000),
	)
	for _, id := range []string{"a", "b", "c"} {
		writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-"+id+".jsonl"),
			codexChildSessionMetaLine(id, ts, "root1", "nick-"+id, "role", 1))
	}

	budget := defaultSubagentReadBudget()
	budget.MaxNodes = 2
	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, budget)
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
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
	byID := codexNodesByID(tree)
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

func TestReadCodexSubagentTreeNoChildrenReturnsNilTree(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", day.Format(time.RFC3339)))

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	if tree != nil {
		t.Fatalf("expected a nil tree for a session with no subagents, got %+v", tree)
	}
}

// TestReadCodexSubagentTreeRespectsChildReadBudgets is 親 plan C3 の完了条件
// 「子の探索で読むのは各 rollout の先頭行だけで、末尾の読み取りは C2 の上限を
// 超えない」の確認。実際に 10MB のファイルを読ませ、末尾 128KB の外にしか
// tool call が無ければ見つからないこと（間接確認）に加えて、
// codexSubagentReadLastTool が返す実際の読み取りバイト数がその上限を超えな
// いこと（直接確認・修正 C8 指摘 R10。旧テストは LastToolName/Summary が空か
// どうかからの推定に留まっていた）も確かめる。
func TestReadCodexSubagentTreeRespectsChildReadBudgets(t *testing.T) {
	day := time.Now()
	_, dayDir := newCodexSubagentFixtureRoot(t, day)
	ts := day.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", ts))

	childPath := filepath.Join(dayDir, "rollout-big.jsonl")
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
	writeOne(codexChildSessionMetaLine("big-child", ts, "root1", "big", "role", 1))
	// An early tool call that sits well outside any tail window once the
	// file grows past 10MB.
	writeOne(codexCustomToolCallLine("exec", "echo this is far from the tail"))
	padLine := map[string]any{
		"type":    "response_item",
		"payload": map[string]any{"type": "message", "text": strings.Repeat("x", 900)},
	}
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

	tree, _, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readCodexSubagentTree error: %v", err)
	}
	byID := codexNodesByID(tree)
	big, ok := byID["big-child"]
	if !ok {
		t.Fatalf("big-child missing from tree: %+v", byID)
	}
	if big.StartedAt == 0 {
		t.Fatal("StartedAt must still be resolved from the first-line-only discovery read even on a 10MB child rollout")
	}
	if big.LastToolName != "" || big.LastToolSummary != "" {
		t.Fatalf("LastToolName/LastToolSummary = %q/%q, want empty: the only tool call is outside the tail window",
			big.LastToolName, big.LastToolSummary)
	}

	// R10 直接確認: 読んだバイト数そのものを数える（結果からの推定ではない）。
	tailBudget := defaultSubagentReadBudget().TailBytes
	_, _, bytesRead := codexSubagentReadLastTool(childPath, tailBudget)
	if bytesRead <= 0 {
		t.Fatal("codexSubagentReadLastTool reported bytesRead=0 reading a 10MB fixture; the stat plumbing is broken")
	}
	if bytesRead > tailBudget {
		t.Fatalf("codexSubagentReadLastTool read %d bytes, want <= TailBytes budget %d", bytesRead, tailBudget)
	}
}

// TestReadCodexSubagentTreePastDayDirScannedOnlyOnce is 子 plan C3 作業内容
// 「一度つながった子のパスは覚えておき、毎回ディレクトリを全部見直さない。見直す
// のは sessions/ 配下の当日ディレクトリの mtime が変わったときだけにする」の
// 直接確認: 過去日ディレクトリへ後から子を追加しても、2 回目の poll では拾わ
// れない（今日のディレクトリだけが mtime 変化で再スキャンされる）。
func TestReadCodexSubagentTreePastDayDirScannedOnlyOnce(t *testing.T) {
	pastDay := time.Now().AddDate(0, 0, -3)
	_, dayDir := newCodexSubagentFixtureRoot(t, pastDay)
	ts := pastDay.Format(time.RFC3339)

	parentPath := filepath.Join(dayDir, "rollout-root.jsonl")
	writeCodexRolloutLines(t, parentPath, codexRootSessionMetaLine("root1", ts))
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-a.jsonl"),
		codexChildSessionMetaLine("a", ts, "root1", "a", "role", 1))

	tree1, state1, err := readCodexSubagentTree(parentPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("first poll error: %v", err)
	}
	byID1 := codexNodesByID(tree1)
	if _, ok := byID1["a"]; !ok {
		t.Fatalf("expected child 'a' discovered on the first poll: %+v", byID1)
	}

	// A file appearing later in the SAME past-day directory must not be
	// picked up: past days are scanned at most once, ever.
	writeCodexRolloutLines(t, filepath.Join(dayDir, "rollout-b.jsonl"),
		codexChildSessionMetaLine("b", ts, "root1", "b", "role", 1))

	tree2, _, err := readCodexSubagentTree(parentPath, time.Time{}, state1, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("second poll error: %v", err)
	}
	byID2 := codexNodesByID(tree2)
	if _, ok := byID2["b"]; ok {
		t.Fatalf("child 'b' added to an already-scanned past day directory must NOT be discovered: %+v", byID2)
	}
	if _, ok := byID2["a"]; !ok {
		t.Fatalf("previously discovered child 'a' must still be present: %+v", byID2)
	}
}

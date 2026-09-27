package hub

// subagent_source_grok_test.go — 親 plan docs/local/plan_subagent-tree-popup.md
// C4／C9 の完了条件をすべて合成データで確認する。実ファイルの本文は一切使わない
// （合成のタイムスタンプ・ID・ツール名・入力のみ）。testGrokUUIDV7 は
// grok_history_handler_test.go（同パッケージ）のヘルパーをそのまま再利用する。
//
// C9 で events.jsonl の実サンプル（21 件・74,471 行、キー名/型名/件数のみ確
// 認）が初めて取れ、その形は updates.jsonl と同じ JSON-RPC 封筒ではなく、
// {"ts":...,"type":"tool_started"|"tool_completed",...,"tool_name":"..."} の
// フラットな形だと判明した。grokToolStartedEvent/grokToolCompletedEvent は
// その確認済みの実形を再現する。

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

func writeGrokJSONLLines(t *testing.T, path string, lines ...map[string]any) {
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

// grokUpdateLine mirrors the real updates.jsonl envelope confirmed 2026-09-23
// (see subagent_source_grok.go's file doc comment):
// {"timestamp":<epoch seconds>,"method":"_x.ai/session/update","params":{"sessionId":...,"update":{...}}}.
// This is the parent's own file's shape — a child's own sibling events.jsonl
// has an unrelated, flat shape (grokToolStartedEvent/grokToolCompletedEvent
// below), confirmed by C9.
func grokUpdateLine(timestampSec int64, sessionID string, update map[string]any) map[string]any {
	return map[string]any{
		"timestamp": timestampSec,
		"method":    "_x.ai/session/update",
		"params": map[string]any{
			"sessionId": sessionID,
			"update":    update,
		},
	}
}

// grokSpawnedUpdate builds a "subagent_spawned" update object, including
// child_session_id — the key C9 added use of (confirmed present 30/30 real
// samples) to resolve the sibling events.jsonl path.
func grokSpawnedUpdate(subagentID, subagentType, description, model, childSessionID string) map[string]any {
	return map[string]any{
		"sessionUpdate":    "subagent_spawned",
		"subagent_id":      subagentID,
		"subagent_type":    subagentType,
		"description":      description,
		"model":            model,
		"child_session_id": childSessionID,
	}
}

func grokFinishedUpdate(subagentID, status string, toolCalls int) map[string]any {
	return map[string]any{
		"sessionUpdate": "subagent_finished",
		"subagent_id":   subagentID,
		"status":        status,
		"tool_calls":    toolCalls,
	}
}

// grokToolStartedEvent/grokToolCompletedEvent mirror the real, flat sibling
// events.jsonl record shape confirmed 2026-09-23 (C9, 21 real samples/74,471
// lines — see subagent_source_grok.go's file doc comment). Neither record
// carries any argument/target field: only ts/type/tool_name(/tool_call_id/
// duration_ms/outcome for tool_completed).
func grokToolStartedEvent(ts time.Time, toolName string) map[string]any {
	return map[string]any{
		"ts":        ts.UTC().Format("2006-01-02T15:04:05.000Z"),
		"type":      "tool_started",
		"tool_name": toolName,
	}
}

func grokToolCompletedEvent(ts time.Time, toolCallID, toolName, outcome string, durationMs int) map[string]any {
	return map[string]any{
		"ts":           ts.UTC().Format("2006-01-02T15:04:05.000Z"),
		"type":         "tool_completed",
		"tool_call_id": toolCallID,
		"duration_ms":  durationMs,
		"tool_name":    toolName,
		"outcome":      outcome,
	}
}

// writeGrokSubagentMeta writes subagents/<id>/meta.json with the real field
// names confirmed 2026-09-23 (subagent_id is the directory name itself, not a
// field inside the file — confirmed 30/30 real samples).
func writeGrokSubagentMeta(t *testing.T, subDir, id string, meta map[string]any) {
	t.Helper()
	dir := filepath.Join(subDir, id)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "meta.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
}

// writeGrokSiblingEvents writes sessionsRoot/<childSessionID>/events.jsonl —
// the real location (親のセッションディレクトリの兄弟) C9 corrected this
// reader to use, replacing the nonexistent subagents/<id>/events.jsonl (R2).
// sessionDir is the *parent's own* session directory (dir containing
// updates.jsonl); sessionsRoot is its parent.
func writeGrokSiblingEvents(t *testing.T, sessionDir, childSessionID string, lines ...map[string]any) {
	t.Helper()
	sessionsRoot := filepath.Dir(sessionDir)
	writeGrokJSONLLines(t, filepath.Join(sessionsRoot, childSessionID, "events.jsonl"), lines...)
}

func grokNodesByID(tree *proto.SubagentTree) map[string]proto.SubagentNode {
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

func TestReadGrokSubagentTreeRunningAndDoneChildrenAreDistinguished(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-1")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-1", grokSpawnedUpdate("running-child", "explore", "look at logs", "grok-4", "running-child-session")),
	)
	// running-child の subagents/<id>/ ディレクトリ自身は存在する（Grok が
	// 直接作る想定）が、中身は空——今の作業は兄弟の events.jsonl から取る。
	if err := os.MkdirAll(filepath.Join(subDir, "running-child"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeGrokSiblingEvents(t, sessionDir, "running-child-session",
		grokToolStartedEvent(now, "grep"),
	)

	writeGrokSubagentMeta(t, subDir, "done-child", map[string]any{
		"parent_session_id":  "parent-session-1",
		"subagent_type":      "general-purpose",
		"description":        "fix bug",
		"status":             "completed",
		"started_at":         now.Add(-time.Minute).Format(time.RFC3339Nano),
		"completed_at":       now.Format(time.RFC3339Nano),
		"tool_calls":         4,
		"effective_model_id": "grok-4",
	})

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)

	running, ok := byID["running-child"]
	if !ok || running.State != "running" || running.Label != "look at logs" || running.Depth != 1 || running.ParentID != "" {
		t.Fatalf("running-child = %+v (ok=%v), want State=running Label=%q Depth=1 ParentID=\"\"", running, ok, "look at logs")
	}
	if running.LastToolName != "grep" {
		t.Fatalf("running-child.LastToolName = %q, want grep (from sibling events.jsonl)", running.LastToolName)
	}
	done, ok := byID["done-child"]
	if !ok || done.State != "done" || done.ToolCalls != 4 || done.Label != "fix bug" || done.Model != "grok-4" {
		t.Fatalf("done-child = %+v (ok=%v), want State=done ToolCalls=4 Label=%q Model=grok-4", done, ok, "fix bug")
	}
}

func TestReadGrokSubagentTreeCancelledStatusMapsToFailedState(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-2")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokSubagentMeta(t, subDir, "cancelled-child", map[string]any{
		"parent_session_id": "parent-session-2",
		"subagent_type":     "general-purpose",
		"description":       "cancelled run",
		"status":            "cancelled",
		"started_at":        now.Add(-time.Minute).Format(time.RFC3339Nano),
		"completed_at":      now.Format(time.RFC3339Nano),
		"tool_calls":        1,
	})

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	failed, ok := byID["cancelled-child"]
	if !ok || failed.State != "failed" {
		t.Fatalf("cancelled-child = %+v (ok=%v), want State=failed (only \"completed\" is non-failing)", failed, ok)
	}
}

func TestReadGrokSubagentTreeUnexpectedParentSessionIsDropped(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-5")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	// subagents/ は parent-session-5 の下で見つかったが、meta 自身の
	// parent_session_id は別のセッションを指している — 木に入れない（子 plan
	// 作業内容「meta に想定外の親子が出たら、その子は木に入れない」）。
	writeGrokSubagentMeta(t, subDir, "mismatched", map[string]any{
		"parent_session_id": "some-other-session",
		"subagent_type":     "general-purpose",
		"description":       "orphaned",
		"status":            "completed",
		"started_at":        now.Format(time.RFC3339Nano),
		"completed_at":      now.Format(time.RFC3339Nano),
		"tool_calls":        1,
	})

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	if tree != nil {
		t.Fatalf("expected a nil tree (only child has a mismatched parent_session_id), got %+v", tree)
	}
}

func TestReadGrokSubagentTreeNoSubagentsDirReturnsNilTree(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-6")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	if err := os.MkdirAll(sessionDir, 0o755); err != nil {
		t.Fatal(err)
	}

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	if tree != nil {
		t.Fatalf("expected a nil tree for a session with no subagents/ dir, got %+v", tree)
	}
}

func TestReadGrokSubagentTreeSinceFilterKeepsRunningDropsOldDone(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-3")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	since := time.Date(2026, 6, 1, 0, 0, 0, 0, time.UTC)
	oldStart := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

	// old-running: 完了/失敗の信号は無く、spawn イベントだけ既知。起動の信号
	// (subagent_spawned) さえ見えていれば、since より前に始まっていても
	// running のまま残る（C9: 「記録が伸びたか」ではなく起動/完了の信号で
	// 決める）。
	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(oldStart.Unix(), "parent-session-3", grokSpawnedUpdate("old-running", "explore", "still going", "grok-4", "old-running-session")),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "old-running"), 0o755); err != nil {
		t.Fatal(err)
	}

	// old-done: since より前に完了しているので出ない。
	writeGrokSubagentMeta(t, subDir, "old-done", map[string]any{
		"parent_session_id": "parent-session-3",
		"subagent_type":     "general-purpose",
		"description":       "long finished",
		"status":            "completed",
		"started_at":        oldStart.Format(time.RFC3339Nano),
		"completed_at":      oldStart.Add(5 * time.Minute).Format(time.RFC3339Nano),
		"tool_calls":        2,
	})

	tree, _, err := readGrokSubagentTree(updatesPath, since, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	if _, ok := byID["old-done"]; ok {
		t.Fatalf("old-done (finished before since) must be filtered out: %+v", byID)
	}
	running, ok := byID["old-running"]
	if !ok || running.State != "running" {
		t.Fatalf("old-running must stay in the tree as running, got %+v (present=%v)", running, ok)
	}
}

func TestReadGrokSubagentTreeCapsAtMaxNodesDropsOldestCompletedFirst(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-4")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	for i, id := range []string{"a", "b", "c"} {
		writeGrokSubagentMeta(t, subDir, id, map[string]any{
			"parent_session_id": "parent-session-4",
			"subagent_type":     "general-purpose",
			"description":       id,
			"status":            "completed",
			"started_at":        now.Add(time.Duration(i) * time.Minute).Format(time.RFC3339Nano),
			"completed_at":      now.Add(time.Duration(i+1) * time.Minute).Format(time.RFC3339Nano),
			"tool_calls":        1,
		})
	}

	budget := defaultSubagentReadBudget()
	budget.MaxNodes = 2
	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, budget)
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	if tree == nil {
		t.Fatal("expected a non-nil tree")
	}
	if len(tree.Nodes) != 2 {
		t.Fatalf("expected 2 nodes (MaxNodes cap), got %d: %+v", len(tree.Nodes), tree.Nodes)
	}
	if tree.Omitted != 1 {
		t.Fatalf("tree.Omitted = %d, want 1", tree.Omitted)
	}
	byID := grokNodesByID(tree)
	if _, ok := byID["a"]; ok {
		t.Fatalf("oldest-finished agent 'a' should have been dropped first: %+v", byID)
	}
	if _, ok := byID["b"]; !ok {
		t.Fatal("agent 'b' should have survived the cap")
	}
	if _, ok := byID["c"]; !ok {
		t.Fatal("agent 'c' should have survived the cap")
	}
}

// TestReadGrokSubagentTreeLastToolNameFromRealEventShapeHasNoSummary confirms
// C9's corrected events.jsonl decode: the real, confirmed schema
// (grokSubagentEventRecord) carries a "tool_name" but no argument/target
// field at all, so LastToolName is populated while LastToolSummary always
// stays empty — there is nothing to allow-list, unlike Claude/Codex.
func TestReadGrokSubagentTreeLastToolNameFromRealEventShapeHasNoSummary(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-7")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-7", grokSpawnedUpdate("tool-child", "general-purpose", "grepping", "grok-4", "tool-child-session")),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "tool-child"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeGrokSiblingEvents(t, sessionDir, "tool-child-session",
		grokToolStartedEvent(now.Add(-time.Second), "grep"),
		grokToolCompletedEvent(now, "call-1", "grep", "success", 42),
	)

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	child, ok := byID["tool-child"]
	if !ok || child.LastToolName != "grep" {
		t.Fatalf("tool-child = %+v (ok=%v), want LastToolName=grep", child, ok)
	}
	if child.LastToolSummary != "" {
		t.Fatalf("LastToolSummary = %q, want empty (real events.jsonl has no argument/target field)", child.LastToolSummary)
	}
}

// TestReadGrokSubagentTreeSkipsNonToolEventTypes confirms the seven other
// real event "type" values (phase_changed, permission_requested, ...) are
// skipped when looking for the most recent tool activity, rather than being
// mistaken for one.
func TestReadGrokSubagentTreeSkipsNonToolEventTypes(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-9")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-9", grokSpawnedUpdate("phase-child", "general-purpose", "thinking", "grok-4", "phase-child-session")),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "phase-child"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeGrokSiblingEvents(t, sessionDir, "phase-child-session",
		grokToolCompletedEvent(now.Add(-time.Second), "call-1", "read_file", "success", 10),
		map[string]any{"ts": now.UTC().Format("2006-01-02T15:04:05.000Z"), "type": "phase_changed", "phase": "thinking"},
	)

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	child, ok := byID["phase-child"]
	if !ok || child.LastToolName != "read_file" {
		t.Fatalf("phase-child = %+v (ok=%v), want LastToolName=read_file (phase_changed has no tool_name and must be skipped)", child, ok)
	}
}

// TestReadGrokSubagentTreeNeverReadsEventsJSONLUnderSubagentsDir guards R2:
// the reader must never read subagents/<id>/events.jsonl (that path does not
// exist in real Grok output — subagents/<id>/ only ever holds meta.json/
// output.json). A sentinel tool name placed there must never surface.
func TestReadGrokSubagentTreeNeverReadsEventsJSONLUnderSubagentsDir(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-10")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-10", grokSpawnedUpdate("wrong-loc-child", "explore", "check wrong path", "grok-4", "wrong-loc-child-session")),
	)
	childDir := filepath.Join(subDir, "wrong-loc-child")
	if err := os.MkdirAll(childDir, 0o755); err != nil {
		t.Fatal(err)
	}
	// R2 が指摘した「実在しない場所」に events.jsonl を置く。読まれたら
	// LastToolName に "should-not-be-read" が漏れる。
	writeGrokJSONLLines(t, filepath.Join(childDir, "events.jsonl"),
		grokToolCompletedEvent(now, "call-1", "should-not-be-read", "success", 1),
	)
	// 兄弟の本物の場所 (sessionsRoot/wrong-loc-child-session/events.jsonl) には
	// 何も置かない — 今のツールは空のままのはず。

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	child, ok := byID["wrong-loc-child"]
	if !ok {
		t.Fatal("wrong-loc-child should still be in the tree (running via spawn signal)")
	}
	if child.State != "running" {
		t.Fatalf("state = %q, want running", child.State)
	}
	if child.LastToolName == "should-not-be-read" {
		t.Fatal("subagents/<id>/events.jsonl (実在しない場所) が読まれている — R2 の再発")
	}
	if child.LastToolName != "" {
		t.Fatalf("LastToolName = %q, want empty (real sibling events.jsonl absent)", child.LastToolName)
	}
}

// TestReadGrokSubagentTreeStaysRunningAcrossPollsWithoutGrowth is C9's
// primary regression test for R1: a child known only via a live
// "subagent_spawned" update (no meta.json) must stay "running" across many
// polls even when nothing about it changes (no new events.jsonl activity, no
// new signal) — the old Grew-driven decision would have dropped it to
// "unknown" as soon as the sibling events.jsonl stopped changing.
func TestReadGrokSubagentTreeStaysRunningAcrossPollsWithoutGrowth(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-11")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-11", grokSpawnedUpdate("long-runner", "explore", "long task", "grok-4", "long-runner-session")),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "long-runner"), 0o755); err != nil {
		t.Fatal(err)
	}
	// 意図的に events.jsonl を一切置かない — 「まだ 1 回もツールを呼んでいない
	// 走行中の子」を再現する（C4 時点のバグでは、この形の子が unknown になっ
	// ていた）。

	var state any
	for i := 0; i < 10; i++ {
		tree, nextState, err := readGrokSubagentTree(updatesPath, time.Time{}, state, defaultSubagentReadBudget())
		if err != nil {
			t.Fatalf("poll %d: readGrokSubagentTree error: %v", i, err)
		}
		state = nextState
		byID := grokNodesByID(tree)
		child, ok := byID["long-runner"]
		if !ok || child.State != "running" {
			t.Fatalf("poll %d: long-runner = %+v (ok=%v), want State=running (no growth must not demote to unknown)", i, child, ok)
		}
	}
}

// TestReadGrokSubagentTreeRunningSurvivesSpawnSignalAgingOutOfParentWindow
// 確認: subagent_spawned が、後続の poll では親の tail scan の record 上限
// （grokSubagentParentReadRecordsMax=500）の外に押し出されても、一度 running
// と判定した子は running のまま保たれる（C9 / R1 — 読み取り状態
// grokSubagentChildCache.Started に覚えておく。親 plan「修正の C の共通の決ま
// り」）。TestReadCodexSubagentTreeRunningSurvivesStartedSignalAgingOutOfParentWindow
// と同じ手法（filler レコードを大量に追記して tail scan の外へ押し出す）。
func TestReadGrokSubagentTreeRunningSurvivesSpawnSignalAgingOutOfParentWindow(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-13")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Unix(), "parent-session-13", grokSpawnedUpdate("windowed-child", "explore", "long task", "grok-4", "windowed-child-session")),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "windowed-child"), 0o755); err != nil {
		t.Fatal(err)
	}

	state1, state1Err := (func() (any, error) {
		tree, nextState, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
		if err != nil {
			return nil, err
		}
		byID := grokNodesByID(tree)
		child, ok := byID["windowed-child"]
		if !ok || child.State != "running" {
			t.Fatalf("first poll: windowed-child = %+v (ok=%v), want State=running", child, ok)
		}
		return nextState, nil
	})()
	if state1Err != nil {
		t.Fatalf("first poll error: %v", state1Err)
	}

	// 501 件の無関係な filler レコードを spawn の後に足し、
	// grokSubagentParentReadRecordsMax(=500) を超えさせる。これで次の poll の
	// tail scan には subagent_spawned レコードがもう含まれない。
	f, err := os.OpenFile(updatesPath, os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 501; i++ {
		filler := grokUpdateLine(now.Unix(), "parent-session-13", map[string]any{"sessionUpdate": "user_message_chunk"})
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

	tree2, _, err := readGrokSubagentTree(updatesPath, time.Time{}, state1, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("second poll error: %v", err)
	}
	byID2 := grokNodesByID(tree2)
	child2, ok := byID2["windowed-child"]
	if !ok || child2.State != "running" {
		t.Fatalf("second poll (spawn signal aged out of window): windowed-child = %+v (ok=%v), want State=running", child2, ok)
	}
}

// TestReadGrokSubagentTreeSubagentFinishedTransitionsToDoneWithoutMeta covers
// the "subagent_finished fires before meta.json is flushed" race this reader
// treats as authoritative on its own (file doc comment).
func TestReadGrokSubagentTreeSubagentFinishedTransitionsToDoneWithoutMeta(t *testing.T) {
	sessionDir := filepath.Join(t.TempDir(), "parent-session-12")
	updatesPath := filepath.Join(sessionDir, "updates.jsonl")
	subDir := filepath.Join(sessionDir, "subagents")
	now := time.Now()

	writeGrokJSONLLines(t, updatesPath,
		grokUpdateLine(now.Add(-time.Minute).Unix(), "parent-session-12", grokSpawnedUpdate("race-child", "explore", "race", "grok-4", "race-child-session")),
		grokUpdateLine(now.Unix(), "parent-session-12", grokFinishedUpdate("race-child", "completed", 3)),
	)
	if err := os.MkdirAll(filepath.Join(subDir, "race-child"), 0o755); err != nil {
		t.Fatal(err)
	}

	tree, _, err := readGrokSubagentTree(updatesPath, time.Time{}, nil, defaultSubagentReadBudget())
	if err != nil {
		t.Fatalf("readGrokSubagentTree error: %v", err)
	}
	byID := grokNodesByID(tree)
	child, ok := byID["race-child"]
	if !ok || child.State != "done" || child.ToolCalls != 3 {
		t.Fatalf("race-child = %+v (ok=%v), want State=done ToolCalls=3", child, ok)
	}
}

// TestResolveGrokSubagentParentPath は agentLogForSession の grok 分岐
// （grokHomeDir + findGrokChatHistory）と同じ解決規則を、provider 名では
// なく渡された情報だけから行うことを確認する。testGrokUUIDV7 は
// grok_history_handler_test.go のヘルパー。
func TestResolveGrokSubagentParentPath(t *testing.T) {
	if _, ok := resolveGrokSubagentParentPath(subagentParentInfo{}); ok {
		t.Fatal("empty info must not resolve a path")
	}
	if _, ok := resolveGrokSubagentParentPath(subagentParentInfo{CWD: "/repo"}); ok {
		t.Fatal("missing GrokHome/HomeDir must not resolve a path")
	}
	if _, ok := resolveGrokSubagentParentPath(subagentParentInfo{CWD: "/repo", HomeDir: `C:\home\x`, StartedAt: "not-a-time"}); ok {
		t.Fatal("unparseable StartedAt must not resolve a path")
	}

	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 9, 1, 30, 0, 0, time.UTC)
	sessionID := testGrokUUIDV7(startedAt.Add(8 * time.Second))
	sessionDir := filepath.Join(grokDir, "sessions", encoded, sessionID)
	if err := os.MkdirAll(sessionDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(sessionDir, "chat_history.jsonl"), []byte("{}\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	got, ok := resolveGrokSubagentParentPath(subagentParentInfo{CWD: cwd, GrokHome: grokDir, StartedAt: startedAt.Format(time.RFC3339)})
	if !ok {
		t.Fatal("expected resolveGrokSubagentParentPath to succeed")
	}
	want := filepath.Join(sessionDir, "updates.jsonl")
	if got != want {
		t.Fatalf("resolveGrokSubagentParentPath = %q, want %q", got, want)
	}
}

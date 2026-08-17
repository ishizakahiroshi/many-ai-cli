package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func defaultWorkflowTaskDetailTestBudget() agentChatReadBudget {
	return agentChatReadBudget{
		MaxBytes:   workflowTaskDetailReadBytesMax,
		MaxRecords: workflowTaskDetailReadRecordsMax,
		Deadline:   time.Now().Add(time.Hour),
	}
}

func workflowToolUseLine(toolUseID string) map[string]any {
	return map[string]any{
		"type": "assistant", "message": map[string]any{
			"role": "assistant",
			"content": []any{
				map[string]any{"type": "tool_use", "id": toolUseID, "name": "Workflow", "input": map[string]any{}},
			},
		},
	}
}

func workflowToolResultLine(toolUseID, text string) map[string]any {
	return map[string]any{
		"type": "user", "message": map[string]any{
			"role": "user",
			"content": []any{
				map[string]any{"type": "tool_result", "tool_use_id": toolUseID, "content": text},
			},
		},
	}
}

func workflowLaunchText(taskID, wfDir string) string {
	return "Workflow launched in background. Task ID: " + taskID + "\n" +
		"Summary: many-ai-cli 全体監査（DBあり版・強度ハイ・調査まで）\n" +
		"Transcript dir: " + wfDir + "\n" +
		"Script file: C:\\Users\\ishiz\\.claude\\workflows\\audit.js"
}

func TestResolveWorkflowTaskIDExtractsFromToolResult(t *testing.T) {
	wfDir := filepath.Join(t.TempDir(), "subagents", "workflows", "wf_ffbe191b-795")
	path := writeAgentChatFixture(t,
		workflowToolUseLine("toolu_1"),
		workflowToolResultLine("toolu_1", workflowLaunchText("w99mcnai0", wfDir)),
	)

	taskID, ok := resolveWorkflowTaskID(path, wfDir, defaultWorkflowTaskDetailTestBudget())
	if !ok || taskID != "w99mcnai0" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want (w99mcnai0, true)", taskID, ok)
	}
}

func TestResolveWorkflowTaskIDPicksMatchingRunAmongMultiple(t *testing.T) {
	oldDir := filepath.Join(t.TempDir(), "wf_old")
	wfDir := filepath.Join(t.TempDir(), "wf_current")
	path := writeAgentChatFixture(t,
		workflowToolUseLine("toolu_old"),
		workflowToolResultLine("toolu_old", workflowLaunchText("old-task", oldDir)),
		workflowToolUseLine("toolu_new"),
		workflowToolResultLine("toolu_new", workflowLaunchText("new-task", wfDir)),
	)

	taskID, ok := resolveWorkflowTaskID(path, wfDir, defaultWorkflowTaskDetailTestBudget())
	if !ok || taskID != "new-task" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want (new-task, true)", taskID, ok)
	}
}

func TestResolveWorkflowTaskIDNoWorkflowToolUse(t *testing.T) {
	path := writeAgentChatFixture(t,
		map[string]any{
			"type": "user", "message": map[string]any{
				"role": "user", "content": []any{map[string]any{"type": "text", "text": "hello"}},
			},
		},
	)

	taskID, ok := resolveWorkflowTaskID(path, "/tmp/wf_x", defaultWorkflowTaskDetailTestBudget())
	if ok || taskID != "" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want unresolved for a transcript with no Workflow tool_use", taskID, ok)
	}
}

func TestResolveWorkflowTaskIDToolResultFormatChanged(t *testing.T) {
	wfDir := filepath.Join(t.TempDir(), "wf_x")
	path := writeAgentChatFixture(t,
		workflowToolUseLine("toolu_1"),
		workflowToolResultLine("toolu_1", "Workflow started. See dashboard for details."),
	)

	taskID, ok := resolveWorkflowTaskID(path, wfDir, defaultWorkflowTaskDetailTestBudget())
	if ok || taskID != "" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want unresolved when tool_result text has no Task ID/Transcript dir lines", taskID, ok)
	}
}

func TestResolveWorkflowTaskIDMismatchedTranscriptDirIsUnresolved(t *testing.T) {
	actualDir := filepath.Join(t.TempDir(), "wf_actual")
	requestedDir := filepath.Join(t.TempDir(), "wf_requested")
	path := writeAgentChatFixture(t,
		workflowToolUseLine("toolu_1"),
		workflowToolResultLine("toolu_1", workflowLaunchText("some-task", actualDir)),
	)

	taskID, ok := resolveWorkflowTaskID(path, requestedDir, defaultWorkflowTaskDetailTestBudget())
	if ok || taskID != "" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want unresolved when Transcript dir does not match the requested wfDir", taskID, ok)
	}
}

func TestResolveWorkflowTaskIDBudgetExceededReturnsUnresolvedWithoutPanic(t *testing.T) {
	wfDir := filepath.Join(t.TempDir(), "wf_x")
	path := writeAgentChatFixture(t,
		workflowToolUseLine("toolu_1"),
		workflowToolResultLine("toolu_1", workflowLaunchText("w1", wfDir)),
	)

	expired := agentChatReadBudget{
		MaxBytes:   workflowTaskDetailReadBytesMax,
		MaxRecords: workflowTaskDetailReadRecordsMax,
		Deadline:   time.Now().Add(-time.Hour),
	}

	defer func() {
		if r := recover(); r != nil {
			t.Fatalf("resolveWorkflowTaskID panicked on an already-expired budget: %v", r)
		}
	}()
	taskID, ok := resolveWorkflowTaskID(path, wfDir, expired)
	if ok || taskID != "" {
		t.Fatalf("resolveWorkflowTaskID = (%q, %v), want unresolved when the read budget is already exhausted", taskID, ok)
	}
}

// TestWorkflowTaskOutputFileShapeExcludesResultAndLogs mirrors
// TestWorkflowJournalEventPrivacyShape: the decode shape must never gain a
// Result or Logs field so encoding/json has nothing to decode those
// script-output bodies into, no matter what future field is added.
func TestWorkflowTaskOutputFileShapeExcludesResultAndLogs(t *testing.T) {
	for _, typ := range []reflect.Type{reflect.TypeOf(workflowTaskOutputFile{}), reflect.TypeOf(workflowTaskOutputEntry{})} {
		for i := 0; i < typ.NumField(); i++ {
			name := typ.Field(i).Name
			if name == "Result" || name == "Logs" {
				t.Fatalf("%s must not declare a %s field: result/logs bodies must never be decoded", typ.Name(), name)
			}
		}
	}
}

func writeWorkflowTaskOutputFixture(t *testing.T, path string, extra string) {
	t.Helper()
	// Modeled on the C1 実測サンプル. result/logs carry a sentinel so the
	// privacy test below can prove neither ever becomes reachable from the
	// decoded state.
	body := `{
		"summary": "many-ai-cli 全体監査",
		"agentCount": 2,
		"logs": ["DO-NOT-LEAK-1234 log line"],
		"result": {"secretMarker": "DO-NOT-LEAK-1234"},
		"workflowProgress": [
			{"type": "workflow_phase", "index": 1, "title": "Find"},
			{"type": "workflow_phase", "index": 2, "title": "Verify"},
			{
				"type": "workflow_agent", "index": 1, "label": "find:bug-hub-core",
				"phaseIndex": 1, "phaseTitle": "Find", "agentId": "ae1", "model": "claude-fable-5",
				"state": "done", "startedAt": 1786966758476, "queuedAt": 1786966758451, "attempt": 1,
				"lastToolName": "StructuredOutput", "lastToolSummary": "did a thing",
				"promptPreview": "prompt excerpt", "lastProgressAt": 1786967707887,
				"tokens": 234803, "toolCalls": 63, "durationMs": 949406,
				"resultPreview": "result excerpt"
			},
			{
				"type": "workflow_agent", "index": 2, "label": "verify:cross-check",
				"phaseIndex": 2, "phaseTitle": "Verify", "agentId": "ae2", "model": "claude-fable-5",
				"state": "running", "startedAt": 1786966760000, "queuedAt": 1786966759000, "attempt": 1,
				"tokens": 100, "toolCalls": 3, "durationMs": 5000
			}
		],
		"totalTokens": 8040813,
		"totalToolCalls": 1637`
	body += extra + "\n}"
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestPollWorkflowTaskDetailFileParsesWorkflowProgress(t *testing.T) {
	path := filepath.Join(t.TempDir(), "w99mcnai0.output")
	writeWorkflowTaskOutputFixture(t, path, "")

	state, reparsed, err := pollWorkflowTaskDetailFile(path, workflowTaskDetailFileState{})
	if err != nil {
		t.Fatal(err)
	}
	if !reparsed || !state.Loaded {
		t.Fatalf("expected first poll to load and report reparsed=true, got reparsed=%v loaded=%v", reparsed, state.Loaded)
	}
	if state.AgentCount != 2 {
		t.Fatalf("AgentCount = %d, want 2", state.AgentCount)
	}
	if len(state.Entries) != 4 {
		t.Fatalf("Entries = %d, want 4 (2 phases + 2 agents)", len(state.Entries))
	}

	progress := buildWorkflowTaskDetailProgress(state.Entries)
	if progress == nil || progress.TaskDetailSource != "task-output" || progress.Source != "task-output" {
		t.Fatalf("buildWorkflowTaskDetailProgress = %#v", progress)
	}
	if len(progress.Phases) != 2 {
		t.Fatalf("Phases = %d, want 2", len(progress.Phases))
	}
	if progress.Phases[0].Title != "Find" || len(progress.Phases[0].Agents) != 1 {
		t.Fatalf("phase 0 = %#v", progress.Phases[0])
	}
	if progress.Phases[1].Title != "Verify" || len(progress.Phases[1].Agents) != 1 {
		t.Fatalf("phase 1 = %#v", progress.Phases[1])
	}
	agent := progress.Phases[0].Agents[0]
	if agent.Label != "find:bug-hub-core" || agent.State != "done" {
		t.Fatalf("agent = %#v", agent)
	}
	if agent.Detail == nil || agent.Detail.Model != "claude-fable-5" || agent.Detail.Tokens != 234803 || agent.Detail.ToolCalls != 63 {
		t.Fatalf("agent.Detail = %#v", agent.Detail)
	}
	if agent.Detail.LastToolSummary != "did a thing" || agent.Detail.PromptPreview != "prompt excerpt" || agent.Detail.ResultPreview != "result excerpt" {
		t.Fatalf("agent.Detail preview fields = %#v", agent.Detail)
	}
}

func TestPollWorkflowTaskDetailFileNeverExposesResultOrLogs(t *testing.T) {
	path := filepath.Join(t.TempDir(), "w1.output")
	writeWorkflowTaskOutputFixture(t, path, "")
	const sentinel = "DO-NOT-LEAK-1234"

	state, _, err := pollWorkflowTaskDetailFile(path, workflowTaskDetailFileState{})
	if err != nil {
		t.Fatal(err)
	}
	entriesJSON, err := json.Marshal(state.Entries)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(entriesJSON), sentinel) {
		t.Fatalf("decoded entries leaked result/logs content: %s", entriesJSON)
	}

	progress := buildWorkflowTaskDetailProgress(state.Entries)
	progressJSON, err := json.Marshal(progress)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(progressJSON), sentinel) {
		t.Fatalf("built WorkflowProgress leaked result/logs content: %s", progressJSON)
	}
}

func TestPollWorkflowTaskDetailFileNoReparseWithoutMtimeChange(t *testing.T) {
	path := filepath.Join(t.TempDir(), "w1.output")
	writeWorkflowTaskOutputFixture(t, path, "")

	state := workflowTaskDetailFileState{}
	reparseCount := 0
	for i := 0; i < 3; i++ {
		var reparsed bool
		var err error
		state, reparsed, err = pollWorkflowTaskDetailFile(path, state)
		if err != nil {
			t.Fatal(err)
		}
		if reparsed {
			reparseCount++
		}
	}
	if reparseCount != 1 {
		t.Fatalf("reparseCount = %d across 3 polls of an unchanged file, want 1", reparseCount)
	}

	future := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(path, future, future); err != nil {
		t.Fatal(err)
	}
	_, reparsed, err := pollWorkflowTaskDetailFile(path, state)
	if err != nil {
		t.Fatal(err)
	}
	if !reparsed {
		t.Fatal("expected a reparse after the file's mtime advanced")
	}
}

func TestPollWorkflowTaskDetailFileSkipsOversizedFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "huge.output")
	data := make([]byte, workflowTaskDetailOutputSizeMax+1024)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}

	state, reparsed, err := pollWorkflowTaskDetailFile(path, workflowTaskDetailFileState{})
	if err != nil {
		t.Fatal(err)
	}
	if reparsed {
		t.Fatal("expected an oversized tasks output file to be skipped, not parsed")
	}
	if state.Loaded {
		t.Fatal("an oversized tasks output file must not be marked Loaded")
	}
}

func TestBuildWorkflowTaskDetailProgressTruncatesPreviewFields(t *testing.T) {
	long := strings.Repeat("あ", workflowTaskDetailTextMax+50)
	entries := []workflowTaskOutputEntry{
		{Type: "workflow_phase", Index: 1, Title: "Find"},
		{Type: "workflow_agent", Index: 1, PhaseIndex: 1, PhaseTitle: "Find", Label: "a", State: "running", PromptPreview: long},
	}
	progress := buildWorkflowTaskDetailProgress(entries)
	if progress == nil || len(progress.Phases) != 1 || len(progress.Phases[0].Agents) != 1 {
		t.Fatalf("progress = %#v", progress)
	}
	preview := progress.Phases[0].Agents[0].Detail.PromptPreview
	if runeLen := len([]rune(preview)); runeLen != workflowTaskDetailTextMax {
		t.Fatalf("PromptPreview rune length = %d, want %d", runeLen, workflowTaskDetailTextMax)
	}
}

func TestBuildWorkflowTaskDetailProgressSynthesizesMissingPhase(t *testing.T) {
	// No workflow_phase entry declares phaseIndex 3; the agent should still
	// surface under a synthetic phase built from its own PhaseTitle rather
	// than being silently dropped.
	entries := []workflowTaskOutputEntry{
		{Type: "workflow_agent", Index: 1, PhaseIndex: 3, PhaseTitle: "Gap", Label: "gap:agent", State: "done"},
	}
	progress := buildWorkflowTaskDetailProgress(entries)
	if progress == nil || len(progress.Phases) != 1 || progress.Phases[0].Title != "Gap" {
		t.Fatalf("progress = %#v", progress)
	}
	if len(progress.Phases[0].Agents) != 1 || progress.Phases[0].Agents[0].Label != "gap:agent" {
		t.Fatalf("phase agents = %#v", progress.Phases[0].Agents)
	}
}

func TestNewWorkflowJournalRunDir(t *testing.T) {
	base := t.TempDir()
	oldPath := filepath.Join(base, "wf_old", "journal.jsonl")
	newPath := filepath.Join(base, "wf_new", "journal.jsonl")
	previous := map[string]workflowJournalFileState{
		oldPath: {ModTime: time.Unix(100, 0)},
	}
	current := map[string]workflowJournalFileState{
		oldPath: {ModTime: time.Unix(100, 0)},
		newPath: {ModTime: time.Unix(200, 0)},
	}
	dir, ok := newWorkflowJournalRunDir(current, previous)
	if !ok || dir != filepath.Dir(newPath) {
		t.Fatalf("newWorkflowJournalRunDir = (%q, %v), want (%q, true)", dir, ok, filepath.Dir(newPath))
	}

	if _, ok := newWorkflowJournalRunDir(current, current); ok {
		t.Fatal("expected no new run dir when current == previous")
	}
}

func TestApplyWorkflowTaskDetailOverlayPreservesCountsOwnedByJournalVT(t *testing.T) {
	out := &proto.WorkflowProgress{Detected: true, Source: "journal", Done: 3, Total: 5, Running: 2, Settled: false}
	detail := &proto.WorkflowProgress{
		TaskDetailSource: "task-output",
		Phases:           []proto.WfPhase{{Title: "Find", Agents: []proto.WfAgent{{Label: "a", State: "done"}}}},
	}
	applyWorkflowTaskDetailOverlay(out, detail)
	if out.Done != 3 || out.Total != 5 || out.Running != 2 || out.Settled {
		t.Fatalf("overlay must not touch counts/settle: %#v", out)
	}
	if out.TaskDetailSource != "task-output" || len(out.Phases) != 1 {
		t.Fatalf("overlay did not apply Phases/TaskDetailSource: %#v", out)
	}

	unchanged := &proto.WorkflowProgress{Done: 1, Total: 2}
	applyWorkflowTaskDetailOverlay(unchanged, nil)
	if unchanged.TaskDetailSource != "" || len(unchanged.Phases) != 0 {
		t.Fatalf("a nil detail must leave out unchanged: %#v", unchanged)
	}
}

package hub

import (
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func TestReattachPreservedStateTransfersLogicalState(t *testing.T) {
	old := &session{
		ParentSessionID:  7,
		Role:             "worker",
		Auto:             true,
		Depth:            2,
		OrchestrationID:  "board-1",
		Activity:         SessionActivity{OutputIdle: false, WorkflowActive: true},
		LastOutputAt:     "2026-08-23T18:00:00Z",
		TranscriptGrewAt: "2026-08-23T18:00:01Z",
		StartedAt:        "2026-08-23T17:00:00Z",
		FirstMessage:     "first",
		LastMessage:      "last",
		workflowVTProgress: &proto.WorkflowProgress{
			Detected: true,
			Done:     1,
			Total:    2,
			Phases:   []proto.WfPhase{{Title: "phase", Agents: []proto.WfAgent{{Label: "agent", State: "running"}}}},
		},
		workflowVTSignature:       "vt-sig",
		workflowVTHasSignal:       true,
		workflowJournalFiles:      map[string]workflowJournalFileState{"journal": {Started: map[string]struct{}{"a": {}}}},
		workflowJournalSessionDir: "session-dir",
		workflowJournalRunning:    true,
		taskDetailWfDir:           "wf-dir",
		taskDetailSessionUUID:     "uuid",
		taskDetailTaskID:          "task",
		taskDetailFileState: workflowTaskDetailFileState{
			Path:    "task.output",
			Loaded:  true,
			Entries: []workflowTaskOutputEntry{{Type: "workflow_agent", Label: "agent"}},
		},
		taskDetailProgress: &proto.WorkflowProgress{
			Source: "task-output",
		},
		gitTurnStartTree:      "tree-start",
		gitTurnStartedAt:      time.Date(2026, 8, 23, 17, 0, 0, 0, time.UTC),
		gitTurns:              []gitTurnSnapshot{{Turn: 1, StartTree: "a", EndTree: "b"}},
		commitMsgAwait:        true,
		commitMsgDeadline:     time.Now().Add(time.Minute),
		commitMsgLang:         "ja",
		commitMsgProgressed:   true,
		initialModelScanBytes: 123,
		initialModelScanDone:  true,
	}
	old.commitMsgBuf.WriteString("commit fragment")
	old.doneMsgBuf.WriteString("done fragment")
	old.workflowScanTimer = time.AfterFunc(time.Hour, func() {})
	old.workflowJournalTimer = time.AfterFunc(time.Hour, func() {})
	old.taskDetailTimer = time.AfterFunc(time.Hour, func() {})

	state := snapshotReattachStateLocked(old)
	stopReattachAsyncStateLocked(old)
	dst := &session{Provider: "claude", State: "running", Activity: SessionActivity{OutputIdle: true}}
	applyReattachPreservedStateLocked(dst, state)

	if dst.ParentSessionID != 7 || dst.Role != "worker" || !dst.Auto || dst.Depth != 2 || dst.OrchestrationID != "board-1" {
		t.Fatalf("orchestration state was not preserved: %+v", dst)
	}
	if dst.Activity != old.Activity || dst.FirstMessage != "first" || dst.LastMessage != "last" {
		t.Fatalf("session display state was not preserved: activity=%+v first=%q last=%q", dst.Activity, dst.FirstMessage, dst.LastMessage)
	}
	if dst.workflowVTProgress == old.workflowVTProgress || dst.workflowVTProgress.Done != 1 || dst.workflowVTSignature != "vt-sig" {
		t.Fatal("workflow progress was not transferred as an independent snapshot")
	}
	dst.workflowJournalFiles["new"] = workflowJournalFileState{}
	if len(old.workflowJournalFiles) != 1 || dst.workflowJournalSessionDir != "session-dir" || !dst.workflowJournalRunning {
		t.Fatal("workflow journal state was not preserved")
	}
	if dst.taskDetailProgress == old.taskDetailProgress || dst.taskDetailTaskID != "task" || len(dst.taskDetailFileState.Entries) != 1 {
		t.Fatal("task detail state was not preserved")
	}
	if dst.gitTurnStartTree != "tree-start" || len(dst.gitTurns) != 1 || dst.gitTurnCaptureInFlight || dst.gitTurnCaptureDone != nil {
		t.Fatal("git turn state was not preserved/reset correctly")
	}
	if got := dst.commitMsgBuf.String(); got != "commit fragment" {
		t.Fatalf("commit marker buffer = %q, want preserved fragment", got)
	}
	if got := dst.doneMsgBuf.String(); got != "done fragment" {
		t.Fatalf("done marker buffer = %q, want preserved fragment", got)
	}
	if dst.workflowBroadcastSignature != "" || !dst.initialModelScanDone || dst.initialModelScanBytes != 123 {
		t.Fatal("reattach delivery reset or model scan state was incorrect")
	}
	if old.workflowScanTimer != nil || old.workflowJournalTimer != nil || old.taskDetailTimer != nil {
		t.Fatal("old timer chain was not stopped during reattach")
	}
}

func TestWrapperCleanupAlreadyDismissedLocked(t *testing.T) {
	s := newTestServer()
	if !wrapperCleanupAlreadyDismissedLocked(s, 1) {
		t.Fatal("empty maps should represent an already dismissed session")
	}
	s.sessions[1] = &session{}
	if wrapperCleanupAlreadyDismissedLocked(s, 1) {
		t.Fatal("a remaining session must be finalized as a disconnect")
	}
	delete(s.sessions, 1)
	s.wrappers[1] = &wrapperConn{}
	if wrapperCleanupAlreadyDismissedLocked(s, 1) {
		t.Fatal("a remaining wrapper must not be treated as a dismissed session")
	}
}

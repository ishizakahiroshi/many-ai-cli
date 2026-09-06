package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

func setupBoardEventTest(t *testing.T, provider string) (*Server, *session, string, string, *[]string) {
	t.Helper()
	s := newTestServer()
	parent := registerTestSession(s, 1, provider)
	parent.OrchestrationID = "notify-board"
	parent.Activity = SessionActivity{OutputIdle: true}
	boardPath := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(boardPath, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	parent.BoardPath = boardPath
	s.registerBoardSession(parent.OrchestrationID, boardPath, parent.ID, "conductor")
	if provider == "codex" {
		parent.vt = newVTBuffer(100, 24)
		parent.vt.Write([]byte("Ask Codex to do anything\r\n? for shortcuts\r\n"))
	}
	frames := []string{}
	s.wrappers[parent.ID] = &wrapperConn{sendFunc: func(raw any) error {
		if msg, ok := raw.(proto.Message); ok && msg.Type == "pty_input" {
			frames = append(frames, string(msg.Data))
		}
		return nil
	}}
	// Keep Enter confirmation deterministic and fast. These tests assert the
	// event body and FIFO state, not the generic submit-enter retry heuristic.
	s.submitEnter = submitEnterTiming{
		idleSettle:    time.Nanosecond,
		minWait:       time.Nanosecond,
		slowMinWait:   time.Nanosecond,
		maxWait:       time.Millisecond,
		poll:          time.Microsecond,
		confirmWindow: time.Microsecond,
	}
	return s, parent, parent.OrchestrationID, boardPath, &frames
}

func queuedBoardEvents(s *Server, boardID string) []boardEvent {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	return append([]boardEvent(nil), s.orchestration.boards[boardID].PendingEvents...)
}

func TestProgressNotificationDoesNotFanOutToSiblingChildren(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifySoft
	parent := registerTestSession(s, 1, "codex")
	parent.OrchestrationID = "progress-board"
	child := registerTestSession(s, 10, "codex")
	sibling := registerTestSession(s, 11, "codex")
	for _, ses := range []*session{child, sibling} {
		ses.ParentSessionID = parent.ID
		ses.OrchestrationID = parent.OrchestrationID
	}
	child.Role = "implementation"
	sibling.Role = "review"
	boardPath := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(boardPath, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.registerBoardSession(parent.OrchestrationID, boardPath, parent.ID, "conductor")
	s.registerBoardChild(parent.OrchestrationID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	s.registerBoardChild(parent.OrchestrationID, boardPath, sibling.ID, parent.ID, sibling.Role, time.Now())
	if err := os.WriteFile(childProgressPath(boardPath, child.ID), []byte(
		"## implementation session=10 2026-09-05T00:00:00Z\nstatus: running\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	s.scanOrchestrationChildFiles(parent.OrchestrationID, time.Now())

	if !parent.BoardNotifyPending {
		t.Fatal("conductor did not receive the progress badge")
	}
	if child.BoardNotifyPending || sibling.BoardNotifyPending {
		t.Fatalf("progress fanned out to children: child=%v sibling=%v", child.BoardNotifyPending, sibling.BoardNotifyPending)
	}
	if got := len(s.pendingInput[sibling.ID]); got != 0 {
		t.Fatalf("sibling pendingInput = %d, want 0", got)
	}
}

func TestBoardEventIsNotOverwrittenByProgress(t *testing.T) {
	s, parent, boardID, _, _ := setupBoardEventTest(t, "claude")
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifyQueueUntilIdle

	s.notifyBoardEvent(boardID, parent.ID, "important completion")
	for i := 0; i < 3; i++ {
		s.notifyBoardSession(boardID, parent.ID, fmt.Sprintf("progress %d", i))
	}

	events := queuedBoardEvents(s, boardID)
	if len(events) != 1 || !strings.Contains(events[0].Text, "important completion") {
		t.Fatalf("events = %#v, want the original completion", events)
	}
	s.orchestration.mu.Lock()
	progress := s.orchestration.boards[boardID].PendingNotices[parent.ID]
	s.orchestration.mu.Unlock()
	if progress != "progress 2" {
		t.Fatalf("progress notice = %q, want latest progress", progress)
	}
}

func TestSoftNotifyStillDeliversDoneEvent(t *testing.T) {
	s, parent, boardID, boardPath, frames := setupBoardEventTest(t, "claude")
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifySoft
	child := registerTestSession(s, 10, "claude")
	child.ParentSessionID = parent.ID
	child.OrchestrationID = boardID
	child.Role = "implementation"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	if err := os.WriteFile(childProgressPath(boardPath, child.ID), []byte(
		"## implementation session=10 2026-09-05T00:00:00Z\nstatus: done\n## DONE implementation session=10\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	s.scanOrchestrationChildFiles(boardID, time.Now())
	if len(queuedBoardEvents(s, boardID)) != 1 {
		t.Fatal("DONE was not queued as an event")
	}
	s.flushBoardEvents(time.Now())

	if got := strings.Join(*frames, ""); !strings.Contains(got, "child complete role=implementation session=10") {
		t.Fatalf("delivered frames missing DONE event: %q", got)
	}
	if len(queuedBoardEvents(s, boardID)) != 0 {
		t.Fatal("DONE event remained queued after delivery")
	}
}

func TestBoardEventWaitsForAwaitingUserThenDelivers(t *testing.T) {
	s, parent, boardID, _, frames := setupBoardEventTest(t, "claude")
	parent.Activity.AwaitingUser = true
	s.notifyBoardEvent(boardID, parent.ID, "answer the child")

	s.flushBoardEvents(time.Now())
	if len(*frames) != 0 || len(queuedBoardEvents(s, boardID)) != 1 {
		t.Fatalf("event escaped AwaitingUser: frames=%q queued=%d", *frames, len(queuedBoardEvents(s, boardID)))
	}

	parent.Activity.AwaitingUser = false
	s.flushBoardEvents(time.Now())
	if !strings.Contains(strings.Join(*frames, ""), "answer the child") || len(queuedBoardEvents(s, boardID)) != 0 {
		t.Fatalf("event was not delivered after AwaitingUser cleared: frames=%q queued=%d", *frames, len(queuedBoardEvents(s, boardID)))
	}
}

func TestBoardEventWaitsForCodexSideThreadThenDeliversOnMain(t *testing.T) {
	s, parent, boardID, _, frames := setupBoardEventTest(t, "codex")
	parent.vt = newVTBuffer(100, 24)
	parent.vt.Write([]byte("Side from main thread · ctrl + / to switch · ctrl + c to close\r\n"))
	s.notifyBoardEvent(boardID, parent.ID, "main-thread event")

	s.flushBoardEvents(time.Now())
	if len(*frames) != 0 || len(queuedBoardEvents(s, boardID)) != 1 {
		t.Fatalf("event escaped Codex side thread: frames=%q queued=%d", *frames, len(queuedBoardEvents(s, boardID)))
	}

	parent.vt = newVTBuffer(100, 24)
	parent.vt.Write([]byte("Ask Codex to do anything\r\n? for shortcuts\r\n"))
	s.flushBoardEvents(time.Now())
	if !strings.Contains(strings.Join(*frames, ""), "main-thread event") || len(queuedBoardEvents(s, boardID)) != 0 {
		t.Fatalf("event was not delivered on Codex main thread: frames=%q queued=%d", *frames, len(queuedBoardEvents(s, boardID)))
	}
}

func TestOrchestrationEventBlockersAndProviderScope(t *testing.T) {
	idle := SessionActivity{OutputIdle: true}
	codexSession := func(screen string) *session {
		ses := &session{Provider: "codex", Activity: idle, vt: newVTBuffer(100, 24)}
		ses.vt.Write([]byte(screen + "\r\n"))
		return ses
	}
	tests := []struct {
		name    string
		session *session
		blocked bool
	}{
		{name: "awaiting user", session: &session{Provider: "claude", Activity: SessionActivity{OutputIdle: true, AwaitingUser: true}}, blocked: true},
		{name: "awaiting approval", session: &session{Provider: "claude", Activity: SessionActivity{OutputIdle: true, AwaitingApproval: true}}, blocked: true},
		{name: "approval visible", session: &session{Provider: "claude", Activity: idle, approvalVisible: true}, blocked: true},
		{name: "initial injection", session: &session{Provider: "claude", Activity: idle, initialInjectPending: true}, blocked: true},
		{name: "workflow active", session: &session{Provider: "claude", Activity: SessionActivity{WorkflowActive: true}}, blocked: true},
		{name: "codex modal", session: codexSession("Update available! Press enter to continue"), blocked: true},
		{name: "codex composer missing", session: codexSession("Codex is starting"), blocked: true},
		{name: "codex main composer", session: codexSession("Ask Codex to do anything"), blocked: false},
		{name: "other provider idle", session: &session{Provider: "claude", Activity: idle}, blocked: false},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got := orchestrationEventBlockedLocked(tc.session, time.Now()) != ""
			if got != tc.blocked {
				t.Fatalf("blocked = %v, want %v", got, tc.blocked)
			}
		})
	}

	other := &session{Provider: "claude", Activity: idle, vt: newVTBuffer(100, 24)}
	other.vt.Write([]byte("Side from main thread\r\n"))
	if reason := orchestrationEventBlockedLocked(other, time.Now()); reason != "" {
		t.Fatalf("Codex-only side-thread signal blocked another provider: %q", reason)
	}
}

func TestQuestionCursorPreventsReplayAndAllowsNextQuestion(t *testing.T) {
	s, parent, boardID, boardPath, frames := setupBoardEventTest(t, "claude")
	// A QUESTION classifies the update as an event even when progress mode is
	// explicit interrupt; it must not leak a generic progress Enter first.
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifyInterrupt
	parent.Activity.AwaitingUser = true
	child := registerTestSession(s, 10, "claude")
	child.ParentSessionID = parent.ID
	child.OrchestrationID = boardID
	child.Role = "implementation"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	progressPath := childProgressPath(boardPath, child.ID)
	content := "## implementation session=10 2026-09-05T00:00:00Z\nstatus: blocked\n## QUESTION implementation session=10\n"
	if err := os.WriteFile(progressPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	s.scanOrchestrationChildFiles(boardID, time.Now())
	if got := len(queuedBoardEvents(s, boardID)); got != 1 {
		t.Fatalf("first QUESTION queued %d events, want 1", got)
	}
	if len(*frames) != 0 {
		t.Fatalf("QUESTION update also injected progress while blocked: %q", *frames)
	}

	content += "## implementation session=10 2026-09-05T00:01:00Z\nstatus: running\n"
	if err := os.WriteFile(progressPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	s.scanOrchestrationChildFiles(boardID, time.Now())
	if got := len(queuedBoardEvents(s, boardID)); got != 1 {
		t.Fatalf("old QUESTION replayed after progress update: events=%d", got)
	}

	content += "## implementation session=10 2026-09-05T00:02:00Z\nstatus: blocked\n## QUESTION implementation session=10\n"
	if err := os.WriteFile(progressPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	s.scanOrchestrationChildFiles(boardID, time.Now())
	events := queuedBoardEvents(s, boardID)
	if len(events) != 2 {
		t.Fatalf("second QUESTION queued total %d events, want 2", len(events))
	}
	for _, event := range events {
		for _, want := range []string{"role=implementation", "session=10", "progress=" + progressPath} {
			if !strings.Contains(event.Text, want) {
				t.Errorf("question event missing %q: %q", want, event.Text)
			}
		}
	}
}

func TestQuestionIDORRejectedAndRecorded(t *testing.T) {
	s, parent, boardID, boardPath, _ := setupBoardEventTest(t, "claude")
	child := registerTestSession(s, 10, "claude")
	sibling := registerTestSession(s, 11, "claude")
	child.ParentSessionID, sibling.ParentSessionID = parent.ID, parent.ID
	child.OrchestrationID, sibling.OrchestrationID = boardID, boardID
	child.Role, sibling.Role = "implementation", "review"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	s.registerBoardChild(boardID, boardPath, sibling.ID, parent.ID, sibling.Role, time.Now())
	if err := os.WriteFile(childProgressPath(boardPath, child.ID), []byte(
		"## implementation session=10 2026-09-05T00:00:00Z\nstatus: blocked\n## QUESTION review session=11\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	s.scanOrchestrationChildFiles(boardID, time.Now())
	if got := len(queuedBoardEvents(s, boardID)); got != 0 {
		t.Fatalf("sibling QUESTION spoof queued %d events, want 0", got)
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "QUESTION rejected") || !strings.Contains(string(data), "claimed_session=11") {
		t.Fatalf("board missing QUESTION rejection evidence: %s", data)
	}
}

func TestSharedBoardQuestionUsesPrecedingWriterForAuthorization(t *testing.T) {
	s, parent, boardID, boardPath, _ := setupBoardEventTest(t, "claude")
	child := registerTestSession(s, 10, "claude")
	sibling := registerTestSession(s, 11, "claude")
	child.ParentSessionID, sibling.ParentSessionID = parent.ID, parent.ID
	child.Role, sibling.Role = "implementation", "review"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	s.registerBoardChild(boardID, boardPath, sibling.ID, parent.ID, sibling.Role, time.Now())
	content := "## implementation session=10 2026-09-05T00:00:00Z\nstatus: blocked\n## QUESTION review session=11\n"
	if err := os.WriteFile(boardPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(boardPath)
	if err != nil {
		t.Fatal(err)
	}

	s.handleBoardChange(boardID, boardPath, info, content, time.Now())
	if got := len(queuedBoardEvents(s, boardID)); got != 0 {
		t.Fatalf("shared-board QUESTION spoof queued %d events, want 0", got)
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "QUESTION rejected") {
		t.Fatalf("shared board missing rejection evidence: %s", data)
	}
}

func TestSharedBoardQuestionDoesNotAlsoInterruptAsProgress(t *testing.T) {
	s, parent, boardID, boardPath, frames := setupBoardEventTest(t, "claude")
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifyInterrupt
	parent.Activity.AwaitingUser = true
	child := registerTestSession(s, 10, "claude")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	content := "## implementation session=10 2026-09-05T00:00:00Z\nstatus: blocked\n## QUESTION implementation session=10\n"
	if err := os.WriteFile(boardPath, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(boardPath)
	if err != nil {
		t.Fatal(err)
	}

	s.handleBoardChange(boardID, boardPath, info, content, time.Now())

	if got := len(queuedBoardEvents(s, boardID)); got != 1 {
		t.Fatalf("shared-board QUESTION queued %d events, want 1", got)
	}
	if len(*frames) != 0 {
		t.Fatalf("shared-board QUESTION also injected generic progress: %q", *frames)
	}
}

func TestRelayOwnedBoardIgnoresGenericEvents(t *testing.T) {
	h := newRelayHarness(t)
	status := h.start()
	run := h.run(status.OrchestrationID)
	before, err := os.ReadFile(run.boardPath)
	if err != nil {
		t.Fatal(err)
	}

	h.s.notifyBoardEvent(status.OrchestrationID, h.parent.ID, "generic event")

	if got := len(queuedBoardEvents(h.s, status.OrchestrationID)); got != 0 {
		t.Fatalf("relay-owned board queued %d generic events, want 0", got)
	}
	after, err := os.ReadFile(run.boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(before) {
		t.Fatal("generic event modified a relay-owned board")
	}
}

func TestNotifyOrchestrationErrorUsesRegisteredBoardEventQueue(t *testing.T) {
	s, parent, boardID, _, _ := setupBoardEventTest(t, "claude")

	s.notifyOrchestrationError(parent.ID, "spawn-timeout", "child did not start")

	events := queuedBoardEvents(s, boardID)
	if len(events) != 1 || !strings.Contains(events[0].Text, "MANY-AI-CLI-ORCHESTRATION-ERROR") {
		t.Fatalf("orchestration error events = %#v, want one queued marker", events)
	}
	if got := len(s.pendingInput[parent.ID]); got != 0 {
		t.Fatalf("orchestration error used generic pendingInput: %d entries", got)
	}
}

func TestBoardEventWithInvalidConductorIsRetainedAsBoardEvidence(t *testing.T) {
	s, parent, boardID, boardPath, _ := setupBoardEventTest(t, "claude")
	child := registerTestSession(s, 10, "claude")
	child.ParentSessionID = parent.ID
	child.OrchestrationID = boardID
	child.Role = "implementation"
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())

	s.notifyBoardEvent(boardID, child.ID, "misdirected important event")

	if got := len(queuedBoardEvents(s, boardID)); got != 0 {
		t.Fatalf("invalid conductor queued %d events, want 0", got)
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "event pending: misdirected important event") {
		t.Fatalf("invalid conductor event has no board evidence: %s", data)
	}
}

func TestBoardEventOverflowIsRecordedNotSilent(t *testing.T) {
	s, parent, boardID, boardPath, _ := setupBoardEventTest(t, "claude")
	delete(s.wrappers, parent.ID)
	for i := 0; i < maxBoardEvents+1; i++ {
		s.notifyBoardEvent(boardID, parent.ID, fmt.Sprintf("event-%03d", i))
	}

	s.orchestration.mu.Lock()
	queued := len(s.orchestration.boards[boardID].PendingEvents)
	overflow := s.orchestration.boards[boardID].EventOverflow[parent.ID]
	s.orchestration.mu.Unlock()
	if queued != maxBoardEvents || overflow != 1 {
		t.Fatalf("queued=%d overflow=%d, want %d/1", queued, overflow, maxBoardEvents)
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	board := string(data)
	if !strings.Contains(board, "event-000") || !strings.Contains(board, fmt.Sprintf("event-%03d", maxBoardEvents)) {
		t.Fatal("board did not retain both the oldest event and the overflow event")
	}

	// Once a queue slot opens, the next poll must promote the overflow counter
	// into an ordered reference rather than merely forgetting the warning.
	s.orchestration.mu.Lock()
	s.orchestration.boards[boardID].PendingEvents = s.orchestration.boards[boardID].PendingEvents[1:]
	s.orchestration.mu.Unlock()
	s.flushBoardEvents(time.Now())
	s.orchestration.mu.Lock()
	events := append([]boardEvent(nil), s.orchestration.boards[boardID].PendingEvents...)
	overflow = s.orchestration.boards[boardID].EventOverflow[parent.ID]
	s.orchestration.mu.Unlock()
	if len(events) != maxBoardEvents || overflow != 0 {
		t.Fatalf("promoted queue=%d overflow=%d, want %d/0", len(events), overflow, maxBoardEvents)
	}
	last := events[len(events)-1].Text
	if !strings.Contains(last, "1 additional events") || !strings.Contains(last, boardPath) {
		t.Fatalf("overflow reference = %q, want count and board path", last)
	}
}

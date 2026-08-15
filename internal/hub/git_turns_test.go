package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func initGitTurnTestRepo(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	run := func(args ...string) {
		t.Helper()
		cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, out)
		}
	}
	run("init")
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("one\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	run("add", "tracked.txt")
	run("-c", "user.name=Test User", "-c", "user.email=test@example.com", "commit", "-m", "initial")
	return dir
}

func waitForGitTurnCapture(t *testing.T, s *Server, sessionID int) {
	t.Helper()
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses == nil || !ses.gitTurnCaptureInFlight {
		s.sessionsMu.Unlock()
		return
	}
	done := ses.gitTurnCaptureDone
	s.sessionsMu.Unlock()
	if done == nil {
		t.Fatal("git turn capture is in flight without a completion channel")
	}
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("timed out waiting for git turn capture")
	}
}

func TestGitTreeDiffCapturesTrackedAndUntrackedChanges(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	startTree, err := writeGitWorktreeTree(ctx, dir)
	if err != nil {
		t.Fatalf("write start tree: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("one\ntwo\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "new.txt"), []byte("new\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	endTree, err := writeGitWorktreeTree(ctx, dir)
	if err != nil {
		t.Fatalf("write end tree: %v", err)
	}
	diff, err := gitTreeDiff(ctx, dir, startTree, endTree)
	if err != nil {
		t.Fatalf("gitTreeDiff: %v", err)
	}
	if diff.Summary.FilesChanged != 2 || diff.Summary.Added != 2 || diff.Summary.Removed != 0 {
		t.Fatalf("summary = %+v, want files=2 added=2 removed=0", diff.Summary)
	}
	if len(diff.Files) != 2 {
		t.Fatalf("files = %d, want 2", len(diff.Files))
	}
	for _, file := range diff.Files {
		if file.Diff == "" {
			t.Errorf("%s has empty diff", file.Path)
		}
	}
}

func TestCaptureGitTurnLifecycle(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	s := newTestServer()
	s.sessions[7] = &session{
		ID:       7,
		Provider: "codex",
		CWD:      dir,
		State:    "running",
		inputMu:  new(sync.Mutex),
	}

	s.captureGitTurnStart(7)
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.captureGitTurnEnd(7, time.Now().Format(time.RFC3339))
	waitForGitTurnCapture(t, s, 7)

	s.sessionsMu.Lock()
	turns := append([]gitTurnSnapshot(nil), s.sessions[7].gitTurns...)
	startTree := s.sessions[7].gitTurnStartTree
	s.sessionsMu.Unlock()
	if len(turns) != 1 {
		t.Fatalf("turn count = %d, want 1", len(turns))
	}
	if turns[0].Turn != 1 || turns[0].Files != 1 || turns[0].Added != 1 || turns[0].Removed != 1 {
		t.Fatalf("turn = %+v, want #1 files=1 +1 -1", turns[0])
	}
	if startTree != "" {
		t.Fatalf("pending start tree = %q, want cleared", startTree)
	}

	s.cfg.Token = "tok"
	turnsReq := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:47777/api/git-turns?session=7&token=tok", nil)
	turnsRec := httptest.NewRecorder()
	s.handleGitTurns(turnsRec, turnsReq)
	if turnsRec.Code != http.StatusOK {
		t.Fatalf("git turns status = %d, body=%s", turnsRec.Code, turnsRec.Body.String())
	}
	var turnsResp gitTurnsResp
	if err := json.Unmarshal(turnsRec.Body.Bytes(), &turnsResp); err != nil {
		t.Fatalf("decode git turns: %v", err)
	}
	if len(turnsResp.Turns) != 1 || turnsResp.Turns[0].Turn != 1 {
		t.Fatalf("git turns response = %+v", turnsResp.Turns)
	}

	diffReq := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:47777/api/git-turn-diff?session=7&turn=1&token=tok", nil)
	diffRec := httptest.NewRecorder()
	s.handleGitTurnDiff(diffRec, diffReq)
	if diffRec.Code != http.StatusOK {
		t.Fatalf("git turn diff status = %d, body=%s", diffRec.Code, diffRec.Body.String())
	}
	var diffResp gitDiffResp
	if err := json.Unmarshal(diffRec.Body.Bytes(), &diffResp); err != nil {
		t.Fatalf("decode git turn diff: %v", err)
	}
	if diffResp.Summary.FilesChanged != 1 || len(diffResp.Files) != 1 {
		t.Fatalf("git turn diff response = %+v", diffResp)
	}
}

func TestNextInputWaitsForPreviousTurnEndBeforeBaselineAndDelivery(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	s := newTestServer()
	ses := &session{
		ID:                     8,
		Provider:               "codex",
		CWD:                    dir,
		State:                  "running",
		inputMu:                new(sync.Mutex),
		gitTurnStartTree:       "previous-turn-start",
		gitTurnStartedAt:       time.Now().Add(-time.Minute),
		gitTurnCaptureInFlight: true,
		gitTurnCaptureDone:     make(chan struct{}),
	}
	s.sessions[8] = ses
	previousDone := ses.gitTurnCaptureDone

	handlerDone := make(chan struct{})
	go func() {
		s.handleInput(proto.Message{SessionID: 8, Text: "next turn\r"})
		close(handlerDone)
	}()

	// The next input must remain before submitInput while the previous end
	// snapshot is unresolved. With no wrapper, submitInput would enqueue it.
	waiterDeadline := time.Now().Add(10 * time.Second)
	for {
		s.sessionsMu.Lock()
		waiters := ses.gitTurnCaptureWaiters
		s.sessionsMu.Unlock()
		if waiters > 0 {
			break
		}
		select {
		case <-handlerDone:
			t.Fatal("next input passed the previous turn-end capture")
		default:
		}
		if time.Now().After(waiterDeadline) {
			t.Fatal("next input did not enter the previous turn-end wait")
		}
		runtime.Gosched()
	}
	s.sessionsMu.Lock()
	if got := len(s.pendingInput[8]); got != 0 {
		s.sessionsMu.Unlock()
		t.Fatalf("pending input count while previous capture is blocked = %d, want 0", got)
	}
	// Deterministically complete the previous end capture. The waiter must then
	// take a fresh start tree before handleInput reaches provider delivery.
	ses.gitTurnStartTree = ""
	ses.gitTurnStartedAt = time.Time{}
	ses.gitTurnCaptureInFlight = false
	ses.gitTurnCaptureDone = nil
	close(previousDone)
	s.sessionsMu.Unlock()

	select {
	case <-handlerDone:
	case <-time.After(10 * time.Second):
		t.Fatal("next input did not resume after previous turn-end capture")
	}
	s.sessionsMu.Lock()
	newBaseline := ses.gitTurnStartTree
	pending := append([]string(nil), s.pendingInput[8]...)
	s.sessionsMu.Unlock()
	if !validRevision(newBaseline) {
		t.Fatalf("next turn baseline = %q, want a captured tree object", newBaseline)
	}
	if len(pending) != 1 || pending[0] != "next turn\r" {
		t.Fatalf("pending provider delivery = %#v, want next input after baseline", pending)
	}
}

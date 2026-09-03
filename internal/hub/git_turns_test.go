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

func TestReusableGitTurnIndexMatchesColdSnapshots(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	indexDir := t.TempDir()
	indexPath := filepath.Join(indexDir, "index")
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	ready := false
	previousHead := ""
	assertMatchesCold := func(stage string) {
		t.Helper()
		cached, head, err := writeGitWorktreeTreeAtIndex(ctx, dir, indexPath, ready, previousHead)
		if err != nil {
			t.Fatalf("%s cached snapshot: %v", stage, err)
		}
		cold, err := writeGitWorktreeTree(ctx, dir)
		if err != nil {
			t.Fatalf("%s cold snapshot: %v", stage, err)
		}
		if cached != cold {
			t.Fatalf("%s cached tree = %s, cold tree = %s", stage, cached, cold)
		}
		ready = true
		previousHead = head
	}

	assertMatchesCold("initial")
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "added.txt"), []byte("added\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	assertMatchesCold("modified-and-added")
	if runtime.GOOS != "windows" {
		if err := os.Chmod(filepath.Join(dir, "added.txt"), 0o700); err != nil {
			t.Fatal(err)
		}
		assertMatchesCold("mode-changed")
	}
	if err := os.Remove(filepath.Join(dir, "tracked.txt")); err != nil {
		t.Fatal(err)
	}
	assertMatchesCold("deleted")

	cmd := exec.Command("git", "-C", dir, "add", "-A")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git add before HEAD change: %v\n%s", err, out)
	}
	cmd = exec.Command("git", "-C", dir, "-c", "user.name=Test User", "-c", "user.email=test@example.com", "commit", "-m", "second")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git commit before HEAD change: %v\n%s", err, out)
	}
	assertMatchesCold("head-changed")
}

func TestGitTurnIndexResetDecision(t *testing.T) {
	tests := []struct {
		name                  string
		ready                 bool
		previousHead, current string
		want                  bool
	}{
		{name: "cold", ready: false, previousHead: "same", current: "same", want: true},
		{name: "same HEAD", ready: true, previousHead: "same", current: "same", want: false},
		{name: "changed HEAD", ready: true, previousHead: "old", current: "new", want: true},
		{name: "unborn remains unborn", ready: true, want: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := gitTurnIndexNeedsReset(tt.ready, tt.previousHead, tt.current); got != tt.want {
				t.Fatalf("gitTurnIndexNeedsReset() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestGitTurnIndexRemovedOnDismiss(t *testing.T) {
	indexDir, err := os.MkdirTemp("", gitTurnIndexTempPrefix)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { removeGitTurnIndexDir(indexDir) })
	if err := os.WriteFile(filepath.Join(indexDir, "index"), []byte("cache"), 0o600); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.sessions[77] = &session{ID: 77, gitTurnIndexDir: indexDir}
	s.handleDismiss(proto.Message{SessionID: 77})
	if _, err := os.Stat(indexDir); !os.IsNotExist(err) {
		t.Fatalf("git turn index directory still exists after dismiss: %v", err)
	}
}

func TestRemoveGitTurnIndexDirRejectsNestedLookalike(t *testing.T) {
	lookalike := filepath.Join(t.TempDir(), gitTurnIndexTempPrefix+"nested")
	if err := os.Mkdir(lookalike, 0o700); err != nil {
		t.Fatal(err)
	}
	removeGitTurnIndexDir(lookalike)
	if _, err := os.Stat(lookalike); err != nil {
		t.Fatalf("nested lookalike should not be removed: %v", err)
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

func TestNextInputWaitsForGitTurnEndCallbackBeforeBaseline(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	s := newTestServer()
	s.sessions[9] = &session{
		ID:       9,
		Provider: "claude",
		CWD:      dir,
		State:    "running",
		inputMu:  new(sync.Mutex),
	}

	s.captureGitTurnStart(9)
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	callbackStarted := make(chan struct{})
	releaseCallback := make(chan struct{})
	callbackDone := make(chan struct{})
	s.captureGitTurnEndWithCallback(9, time.Now().Format(time.RFC3339), func(gitTurnSnapshot) {
		close(callbackStarted)
		<-releaseCallback
		close(callbackDone)
	})
	select {
	case <-callbackStarted:
	case <-time.After(10 * time.Second):
		t.Fatal("timed out waiting for Git turn callback")
	}

	nextBaselineDone := make(chan struct{})
	go func() {
		s.captureGitTurnStart(9)
		close(nextBaselineDone)
	}()
	select {
	case <-nextBaselineDone:
		t.Fatal("next baseline overtook the previous Git turn callback")
	case <-time.After(100 * time.Millisecond):
	}

	close(releaseCallback)
	select {
	case <-callbackDone:
	case <-time.After(10 * time.Second):
		t.Fatal("timed out waiting for Git turn callback release")
	}
	select {
	case <-nextBaselineDone:
	case <-time.After(10 * time.Second):
		t.Fatal("timed out waiting for next Git baseline")
	}
	waitForGitTurnCapture(t, s, 9)
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[9]
	if len(ses.gitTurns) != 1 || ses.gitTurns[0].Files != 1 || ses.gitTurnStartTree == "" {
		t.Fatalf("Git turn state after callback = turns=%+v start_tree=%q", ses.gitTurns, ses.gitTurnStartTree)
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

func TestGitTurnEndCaptureSurvivesReattach(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	s := newTestServer()
	old := &session{
		ID:       12,
		Provider: "codex",
		CWD:      dir,
		State:    "running",
		inputMu:  new(sync.Mutex),
	}
	s.sessions[12] = old

	s.captureGitTurnStart(12)
	s.sessionsMu.Lock()
	if old.gitTurnStartTree == "" || old.gitTurnIndexDir == "" {
		s.sessionsMu.Unlock()
		t.Fatal("start capture did not record a baseline tree and index")
	}
	s.sessionsMu.Unlock()
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("changed-across-reattach\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	// Simulate an in-flight end worker that still holds the pre-reattach
	// pointer: mark capture pending on A, replace A with B (preserving the
	// start tree / index / completion channel), then run the worker on A.
	s.sessionsMu.Lock()
	startTree := old.gitTurnStartTree
	startedAt := old.gitTurnStartedAt
	captureDone := make(chan struct{})
	old.gitTurnCaptureInFlight = true
	old.gitTurnCaptureDone = captureDone
	state := snapshotReattachStateLocked(old)
	s.sessionsMu.Unlock()

	replacement := &session{
		ID:       12,
		Provider: "codex",
		CWD:      dir,
		State:    "running",
		inputMu:  new(sync.Mutex),
	}
	applyReattachPreservedStateLocked(replacement, state)
	s.sessionsMu.Lock()
	s.sessions[12] = replacement
	s.sessionsMu.Unlock()

	s.captureGitTurnEndWorker(12, time.Now().Format(time.RFC3339), old, captureDone, startTree, startedAt, nil)

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[12]
	if ses == old {
		t.Fatal("expected the replacement session to be live")
	}
	if len(ses.gitTurns) != 1 {
		t.Fatalf("turn count = %d, want 1 (end snapshot dropped across reattach)", len(ses.gitTurns))
	}
	if ses.gitTurns[0].StartTree != startTree || ses.gitTurns[0].EndTree == "" {
		t.Fatalf("completed turn = %+v, want start=%s and a non-empty end tree", ses.gitTurns[0], startTree)
	}
	if ses.gitTurnStartTree != "" {
		t.Fatalf("pending start tree = %q, want cleared", ses.gitTurnStartTree)
	}
	if ses.gitTurnCaptureInFlight || ses.gitTurnCaptureDone != nil {
		t.Fatal("replacement still has an in-flight git turn capture")
	}
}

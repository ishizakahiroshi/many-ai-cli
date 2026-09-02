package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

func orchestrationRequest(method, path string, body any) *http.Request {
	var buf bytes.Buffer
	if body != nil {
		_ = json.NewEncoder(&buf).Encode(body)
	}
	req := httptest.NewRequest(method, path, &buf)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	req.Header.Set("Content-Type", "application/json")
	return req
}

func Test_handleSpawnChild_validationAndLimits(t *testing.T) {
	tests := []struct {
		name string
		body spawnChildRequest
		want int
	}{
		{
			name: "invalid role",
			body: spawnChildRequest{Provider: "codex", Role: ""},
			want: http.StatusBadRequest,
		},
		{
			name: "invalid provider",
			body: spawnChildRequest{Provider: "shell", Role: "tester"},
			want: http.StatusBadRequest,
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			s := newTestServer()
			s.cfg.Hub.AllowLoopbackWithoutToken = true
			s.cfg.Orchestration.MaxDepth = 1
			s.cfg.Orchestration.MaxChildrenPerParent = 4
			s.cfg.Orchestration.MaxTotalSessions = 16
			parent := registerTestSession(s, 1, "codex")
			parent.CWD = t.TempDir()
			rr := httptest.NewRecorder()
			s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/spawn-child", tc.body))
			if rr.Code != tc.want {
				t.Fatalf("status = %d, want %d, body=%s", rr.Code, tc.want, rr.Body.String())
			}
		})
	}

	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Orchestration.MaxDepth = 1
	s.cfg.Orchestration.MaxChildrenPerParent = 4
	s.cfg.Orchestration.MaxTotalSessions = 16
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()
	parent.Depth = 1
	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/spawn-child", spawnChildRequest{Provider: "codex", Role: "tester"}))
	if rr.Code != http.StatusTooManyRequests {
		t.Fatalf("max depth status = %d, want %d, body=%s", rr.Code, http.StatusTooManyRequests, rr.Body.String())
	}
}

func Test_handleSessionInject_selfLoop(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	registerTestSession(s, 1, "codex")

	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/inject", injectRequest{
		Text:          "hello",
		FromSessionID: 1,
		PressEnter:    true,
	}))
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d, body=%s", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
}

func Test_detectBoardDoneEvents(t *testing.T) {
	text := strings.Join([]string{
		"## implementer session=10 2026-06-25T00:00:00Z",
		"work",
		"## DONE implementer session=10",
		"## DONE tester",
		"## DONE bad role",
		"## reviewer session=11",
	}, "\n")
	got := detectBoardDoneEvents(text)
	want := []boardDoneEvent{
		{Role: "implementer", SessionID: 10},
		{Role: "tester"},
		{Role: "bad"},
	}
	if len(got) != len(want) {
		t.Fatalf("len = %d, want %d: %#v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("event[%d] = %#v, want %#v", i, got[i], want[i])
		}
	}
}

func Test_detectLastBoardWriter_skipsDone(t *testing.T) {
	text := "## implementer session=10 2026-06-25T00:00:00Z\nbody\n## DONE implementer session=10\n"
	got := detectLastBoardWriter(text)
	if got.Role != "implementer" || got.SessionID != 10 {
		t.Fatalf("writer = %#v, want implementer session=10", got)
	}
}

func Test_detectLastBoardWriter_skipsSuccess(t *testing.T) {
	text := "## implementer session=10 2026-06-25T00:00:00Z\nbody\n## SUCCESS implementer session=10\n"
	got := detectLastBoardWriter(text)
	if got.Role != "implementer" || got.SessionID != 10 {
		t.Fatalf("writer = %#v, want implementer session=10 after SUCCESS", got)
	}
}

func TestReserveOrchestrationConductorMergesWorktreeMetadata(t *testing.T) {
	s := newTestServer()
	const label = "conductor-worktree"
	worktree := normalWorktree{Path: `C:\repo\.many-ai-cli\worktrees\child`, ParentDir: `C:\repo`, Branch: "orch/child", Created: true}
	s.orchestration.pending[label] = pendingChild{
		NormalWorktree:  worktree,
		WorktreeCleanup: worktreeCleanupDelete,
		WorktreeBranch:  worktree.Branch,
		SpawnedAt:       time.Unix(1, 0),
	}

	id := s.reserveOrchestrationConductor(label, nil)
	s.orchestration.mu.Lock()
	got := s.orchestration.pending[label]
	s.orchestration.mu.Unlock()
	if got.OrchestrationID != id {
		t.Fatalf("orchestration id = %q, want %q", got.OrchestrationID, id)
	}
	if got.NormalWorktree != worktree || got.WorktreeCleanup != worktreeCleanupDelete || got.WorktreeBranch != worktree.Branch {
		t.Fatalf("worktree metadata was replaced: %+v", got)
	}
}

func TestOrchestrationChildProcess(t *testing.T) {
	if os.Getenv("MANY_AI_CLI_TEST_CHILD") != "1" {
		return
	}
	for {
		time.Sleep(time.Second)
	}
}

func TestTerminateUnregisteredWrapKillsProcess(t *testing.T) {
	cmd := exec.Command(os.Args[0], "-test.run=^TestOrchestrationChildProcess$")
	cmd.Env = append(os.Environ(), "MANY_AI_CLI_TEST_CHILD=1")
	if err := cmd.Start(); err != nil {
		t.Fatalf("start child: %v", err)
	}
	terminateUnregisteredWrap(cmd)

	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		_ = cmd.Process.Kill()
		t.Fatal("unregistered wrap process did not exit after kill")
	}
}

func Test_handleBoardChange_doneOnlyMatchingSession(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	childA := registerTestSession(s, 10, "codex")
	childB := registerTestSession(s, 11, "codex")
	for _, child := range []*session{childA, childB} {
		child.ParentSessionID = parent.ID
		child.Role = "implementer"
		child.OrchestrationID = "s1"
		child.State = "running"
	}
	path := filepath.Join(t.TempDir(), "board.md")
	content := "## implementer session=10 2026-06-25T00:00:00Z\nwork\n## DONE implementer session=10\n"
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	s.registerBoardSession("s1", path, parent.ID, "conductor")
	s.registerBoardChild("s1", path, 10, parent.ID, "implementer", time.Now())
	s.registerBoardChild("s1", path, 11, parent.ID, "implementer", time.Now())

	s.handleBoardChange("s1", path, info, content, time.Now())

	if childA.State != "done" {
		t.Fatalf("childA state = %q, want done", childA.State)
	}
	if childB.State == "done" {
		t.Fatalf("childB state = done, want unchanged")
	}
}

func TestBoardDoneIDORRejected(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	childA := registerTestSession(s, 10, "codex")
	childB := registerTestSession(s, 11, "codex")
	for _, child := range []*session{childA, childB} {
		child.ParentSessionID = parent.ID
		child.OrchestrationID = "idor-board"
		child.State = "running"
	}
	childA.Role = "implementer"
	childB.Role = "reviewer"
	path := filepath.Join(t.TempDir(), "board.md")
	content := "## implementer session=10 2026-06-25T00:00:00Z\nwork\n## DONE reviewer session=11\n"
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	s.registerBoardSession("idor-board", path, parent.ID, "conductor")
	s.registerBoardChild("idor-board", path, childA.ID, parent.ID, childA.Role, time.Now())
	s.registerBoardChild("idor-board", path, childB.ID, parent.ID, childB.Role, time.Now())

	s.handleBoardChange("idor-board", path, info, content, time.Now())
	if childA.State == "done" || childB.State == "done" {
		t.Fatalf("sibling DONE spoof changed state: A=%q B=%q", childA.State, childB.State)
	}
}

func Test_notifyBoardSession_queueUntilOutputIdle(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifyQueueUntilIdle
	conductor := registerTestSession(s, 1, "codex")
	conductor.OrchestrationID = "s1"
	conductor.lastOutputAt = time.Now()
	path := filepath.Join(t.TempDir(), "board.md")
	s.registerBoardSession("s1", path, conductor.ID, "conductor")

	s.notifyBoardSession("s1", conductor.ID, "board update one")
	s.notifyBoardSession("s1", conductor.ID, "board update latest")

	s.orchestration.mu.Lock()
	queued := s.orchestration.boards["s1"].PendingNotices[conductor.ID]
	s.orchestration.mu.Unlock()
	if queued != "board update latest" {
		t.Fatalf("queued notice = %q, want latest update", queued)
	}
	if !conductor.BoardNotifyPending {
		t.Fatal("BoardNotifyPending = false, want true while queued")
	}

	// Recent output keeps the notification queued.
	s.flushQueuedBoardNotices(time.Now())
	s.orchestration.mu.Lock()
	_, stillQueued := s.orchestration.boards["s1"].PendingNotices[conductor.ID]
	s.orchestration.mu.Unlock()
	if !stillQueued {
		t.Fatal("queue flushed while conductor still had recent output")
	}

	// The temporary server has no wrapper; flush therefore leaves the text in
	// normal pending input, but consumes the board-notification queue exactly once.
	// P-47: queue flush uses the three-axis idle predicate rather than a
	// duplicate output-age threshold.
	conductor.Activity = SessionActivity{OutputIdle: true}
	s.flushQueuedBoardNotices(time.Now())
	s.orchestration.mu.Lock()
	_, stillQueued = s.orchestration.boards["s1"].PendingNotices[conductor.ID]
	s.orchestration.mu.Unlock()
	if stillQueued || conductor.BoardNotifyPending {
		t.Fatalf("queued=%v pendingBadge=%v, want both false after idle flush", stillQueued, conductor.BoardNotifyPending)
	}
}

func Test_notifyBoardSession_softNotifyDoesNotQueueEnter(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.BoardNotifyMode = config.BoardNotifySoft
	conductor := registerTestSession(s, 1, "codex")
	conductor.OrchestrationID = "s1"
	path := filepath.Join(t.TempDir(), "board.md")
	s.registerBoardSession("s1", path, conductor.ID, "conductor")

	s.notifyBoardSession("s1", conductor.ID, "board update")
	if !conductor.BoardNotifyPending {
		t.Fatal("soft-notify did not set the board update badge")
	}
	s.orchestration.mu.Lock()
	queued := len(s.orchestration.boards["s1"].PendingNotices)
	s.orchestration.mu.Unlock()
	if queued != 0 {
		t.Fatalf("soft-notify queued %d Enter notifications, want 0", queued)
	}
}

func Test_prepareChildWorktree_nonGit(t *testing.T) {
	s := newTestServer()
	cwd := t.TempDir()
	gotCWD, branch, note := s.prepareChildWorktree(cwd, "s1", "tester", config.OrchestrationConfig{WorktreeAuto: boolPtr(true), WorktreeDirRoot: ".many-ai-cli/worktrees"})
	if gotCWD != cwd {
		t.Fatalf("cwd = %q, want %q", gotCWD, cwd)
	}
	if branch != "" {
		t.Fatalf("branch = %q, want empty", branch)
	}
	if !strings.Contains(note, "not a git repository") {
		t.Fatalf("note = %q, want non-git skip", note)
	}
}

// C3 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): 症状B（-cwd 無視）の
// 確認。git リポジトリなら worktree が実際に切られ、子 cwd は <cwd>/.many-ai-cli/worktrees/
// <orchestration id>/<role> になる。2回目の呼び出しは既存の worktree を再利用する。
func Test_prepareChildWorktree_gitRepo(t *testing.T) {
	s := newTestServer()
	repo := newRelayTestRepo(t)
	cfg := config.OrchestrationConfig{WorktreeAuto: boolPtr(true), WorktreeDirRoot: ".many-ai-cli/worktrees"}

	gotCWD, branch, note := s.prepareChildWorktree(repo, "o1", "review", cfg)
	wantCWD := filepath.Join(repo, ".many-ai-cli", "worktrees", "o1", "review")
	if gotCWD != wantCWD {
		t.Fatalf("cwd = %q, want %q", gotCWD, wantCWD)
	}
	if branch != "orch/o1/review" {
		t.Fatalf("branch = %q, want orch/o1/review", branch)
	}
	if !strings.Contains(note, "worktree created") {
		t.Fatalf("note = %q, want worktree created", note)
	}
	if _, err := os.Stat(gotCWD); err != nil {
		t.Fatalf("worktree directory was not created: %v", err)
	}

	gotCWD2, branch2, note2 := s.prepareChildWorktree(repo, "o1", "review", cfg)
	if gotCWD2 != wantCWD || branch2 != branch {
		t.Fatalf("reuse mismatch: cwd=%q branch=%q, want %q / %q", gotCWD2, branch2, wantCWD, branch)
	}
	if !strings.Contains(note2, "worktree reuse") {
		t.Fatalf("note2 = %q, want worktree reuse", note2)
	}
}

// -cwd が無視されているなら、親と別のリポジトリを渡しても worktree が同じ場所にできて
// しまう。実際には each -cwd がそれぞれの根の下に worktree を作ることを固定する
// （観測7では親と同じリポジトリだったため「無視された」と区別が付かなかった）。
func Test_prepareChildWorktree_differentRepoRoots(t *testing.T) {
	s := newTestServer()
	repoA := newRelayTestRepo(t)
	repoB := newRelayTestRepo(t)
	cfg := config.OrchestrationConfig{WorktreeAuto: boolPtr(true), WorktreeDirRoot: ".many-ai-cli/worktrees"}

	cwdA, _, _ := s.prepareChildWorktree(repoA, "o2", "review", cfg)
	cwdB, _, _ := s.prepareChildWorktree(repoB, "o2", "review", cfg)
	if !strings.HasPrefix(cwdA, repoA) {
		t.Fatalf("cwdA = %q, want under repoA %q", cwdA, repoA)
	}
	if !strings.HasPrefix(cwdB, repoB) {
		t.Fatalf("cwdB = %q, want under repoB %q", cwdB, repoB)
	}
	if cwdA == cwdB {
		t.Fatalf("-cwd を変えても同じ worktree パスになった: %q", cwdA)
	}
}

// C3: SameTree が nil（未指定）なら既定どおり worktree が勝つ。
func Test_preparePromptAndWorktree_defaultUsesWorktree(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	repo := newRelayTestRepo(t)
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = repo
	cfg := config.OrchestrationConfig{WorktreeAuto: boolPtr(true), WorktreeDirRoot: ".many-ai-cli/worktrees"}
	body := spawnChildRequest{Role: "review", CWD: repo}

	prep, err := s.preparePromptAndWorktree(parent.ID, parent, body, cfg)
	if err != nil {
		t.Fatalf("preparePromptAndWorktree: %v", err)
	}
	wantCWD := filepath.Join(repo, ".many-ai-cli", "worktrees", prep.orchestrationID, "review")
	if prep.childCWD != wantCWD {
		t.Fatalf("childCWD = %q, want %q", prep.childCWD, wantCWD)
	}
	if prep.absoluteCWD != repo {
		t.Fatalf("absoluteCWD = %q, want %q (requested cwd, before worktree substitution)", prep.absoluteCWD, repo)
	}
	if prep.branch == "" {
		t.Fatal("branch は空でないはず（worktree が作られている）")
	}
}

// C3: SameTree=true を明示すると worktree を使わず、要求 cwd をそのまま子 cwd にする。
func Test_preparePromptAndWorktree_sameTreeSkipsWorktree(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	repo := newRelayTestRepo(t)
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = repo
	cfg := config.OrchestrationConfig{WorktreeAuto: boolPtr(true), WorktreeDirRoot: ".many-ai-cli/worktrees"}
	sameTree := true
	body := spawnChildRequest{Role: "review", CWD: repo, SameTree: &sameTree}

	prep, err := s.preparePromptAndWorktree(parent.ID, parent, body, cfg)
	if err != nil {
		t.Fatalf("preparePromptAndWorktree: %v", err)
	}
	if prep.childCWD != repo {
		t.Fatalf("childCWD = %q, want %q (same-tree should skip the worktree)", prep.childCWD, repo)
	}
	if prep.branch != "" {
		t.Fatalf("branch = %q, want empty when same-tree skips the worktree", prep.branch)
	}
	if _, err := os.Stat(filepath.Join(repo, ".many-ai-cli", "worktrees")); !os.IsNotExist(err) {
		t.Fatalf("worktree directory should not have been created with -same-tree (stat err=%v)", err)
	}
	board, err := os.ReadFile(prep.boardPath)
	if err != nil {
		t.Fatalf("read board: %v", err)
	}
	if !strings.Contains(string(board), "worktree skip: requested by --same-tree") {
		t.Fatalf("board missing the same-tree skip note:\n%s", board)
	}
}

func Test_scanOrchestrationBoards_timeoutMarksChild(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.ChildTimeoutSeconds = 1
	s.cfg.Orchestration.IdleDoneThresholdSec = 0
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "tester"
	child.OrchestrationID = "s1"
	child.State = "running"
	path := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(path, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	spawnedAt := time.Now().Add(-2 * time.Second)
	s.registerBoardSession("s1", path, parent.ID, "conductor")
	s.registerBoardChild("s1", path, child.ID, parent.ID, child.Role, spawnedAt)

	s.scanOrchestrationBoards()

	if child.State != "timeout" {
		t.Fatalf("child state = %q, want timeout", child.State)
	}
}

func Test_completeOrchestrationChildOnSessionEnd_marksDoneWithoutMarker(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	path := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(path, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.registerBoardSession("s1", path, parent.ID, "conductor")
	s.registerBoardChild("s1", path, child.ID, parent.ID, child.Role, time.Now())

	s.completeOrchestrationChildOnSessionEnd(child.ID, "completed")

	s.orchestration.mu.Lock()
	done := s.orchestration.boards["s1"].Done[child.ID]
	s.orchestration.mu.Unlock()
	if !done {
		t.Fatal("child EOF did not mark orchestration completion")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "completed without DONE marker") {
		t.Fatalf("board missing EOF completion record: %s", data)
	}
}

func Test_detectBoardDoneEvents_acceptsSuccess(t *testing.T) {
	events := detectBoardDoneEvents("## SUCCESS review session=42\n")
	if len(events) != 1 || events[0].Role != "review" || events[0].SessionID != 42 {
		t.Fatalf("SUCCESS events = %#v, want review/session 42", events)
	}
}

func Test_handleSendChild(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	child.State = "running"
	s.wrappers[child.ID] = newWrapperConn(&websocket.Conn{})
	boardPath := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(boardPath, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	parent.BoardPath = boardPath

	// 生存子あり → 200 + board へ宛先付き conductor 記帳
	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/send-child", sendChildRequest{Role: "implementation", Text: "C2 を実装して"}))
	if rr.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body=%s", rr.Code, rr.Body.String())
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "@implementation session=10 への指示:") {
		t.Fatalf("board missing addressed conductor entry: %s", string(data))
	}
	if !strings.Contains(string(data), "C2 を実装して") {
		t.Fatalf("board missing instruction text: %s", string(data))
	}

	// 該当 role の生存子なし → 404
	rr = httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/send-child", sendChildRequest{Role: "review", Text: "レビューして"}))
	if rr.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404, body=%s", rr.Code, rr.Body.String())
	}

	// done の子は生存子扱いしない → 404
	child.State = "done"
	rr = httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/send-child", sendChildRequest{Role: "implementation", Text: "追加で"}))
	if rr.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404 (done child), body=%s", rr.Code, rr.Body.String())
	}
}

func Test_handleSpawnChild_duplicateRoleGuard(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Orchestration.MaxDepth = 1
	s.cfg.Orchestration.MaxChildrenPerParent = 4
	s.cfg.Orchestration.MaxTotalSessions = 16
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	child.State = "running"
	// liveChildForRole は wrapper 接続中を必須とするため、ガード試験でも stub を置く
	s.wrappers[child.ID] = newWrapperConn(&websocket.Conn{})

	// 同 role の生存子あり → 409 + send 案内（--force なし。done 後に spawn が通ることは
	// liveChildForRole のユニットテストで担保する — handler を通すと実 spawn が走るため）
	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/spawn-child", spawnChildRequest{Provider: "codex", Role: "implementation"}))
	if rr.Code != http.StatusConflict {
		t.Fatalf("status = %d, want 409, body=%s", rr.Code, rr.Body.String())
	}
	if !strings.Contains(rr.Body.String(), "orchestrate send") {
		t.Fatalf("409 detail should suggest orchestrate send, body=%s", rr.Body.String())
	}
	_ = child
}

func Test_liveChildForRole_picksLatestAndSkipsDone(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	children := map[int]*session{}
	for _, id := range []int{10, 12} {
		child := registerTestSession(s, id, "codex")
		child.ParentSessionID = parent.ID
		child.Role = "implementation"
		child.State = "running"
		children[id] = child
		// live 判定は wrapper 接続中も必須
		s.wrappers[id] = newWrapperConn(&websocket.Conn{})
	}
	got := s.liveChildForRole(parent.ID, "implementation")
	if got == nil || got.ID != 12 {
		t.Fatalf("liveChildForRole = %#v, want session 12", got)
	}
	// done の子は候補から外れ、残る生存子が選ばれる
	children[12].State = "done"
	got = s.liveChildForRole(parent.ID, "implementation")
	if got == nil || got.ID != 10 {
		t.Fatalf("liveChildForRole after done = %#v, want session 10", got)
	}
	// completed / disconnected / wrapper 無しも終端扱い
	children[10].State = "completed"
	if s.liveChildForRole(parent.ID, "implementation") != nil {
		t.Fatalf("liveChildForRole with completed should be nil")
	}
	children[10].State = "running"
	delete(s.wrappers, 10)
	if s.liveChildForRole(parent.ID, "implementation") != nil {
		t.Fatalf("liveChildForRole without wrapper should be nil")
	}
	s.wrappers[10] = newWrapperConn(&websocket.Conn{})
	// 全員 done なら nil（= spawn ガードが解除され新規 spawn が通る条件）
	children[10].State = "done"
	if s.liveChildForRole(parent.ID, "implementation") != nil {
		t.Fatalf("liveChildForRole with all done should be nil")
	}
	if s.liveChildForRole(parent.ID, "review") != nil {
		t.Fatalf("liveChildForRole for absent role should be nil")
	}
}

func Test_checkOrchestrationChildTimers_idleUsesPtyOutput(t *testing.T) {
	s := newTestServer()
	cfg := config.OrchestrationConfig{IdleDoneThresholdSec: 60}
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "tester"
	path := filepath.Join(t.TempDir(), "board.md")
	old := time.Now().Add(-10 * time.Minute)
	s.registerBoardSession("s1", path, parent.ID, "conductor")
	s.registerBoardChild("s1", path, child.ID, parent.ID, child.Role, old)

	idleWarned := func() bool {
		s.orchestration.mu.Lock()
		defer s.orchestration.mu.Unlock()
		return s.orchestration.boards["s1"].IdleWarned[child.ID]
	}

	// board 記帳は閾値超過だが PTY 出力が新しい → 作業中とみなし警告しない
	child.lastOutputAt = time.Now()
	s.checkOrchestrationChildTimers("s1", time.Now(), cfg)
	if idleWarned() {
		t.Fatalf("IdleWarned = true, want false (fresh PTY output should suppress idle warning)")
	}

	// board も PTY も閾値超過 → 警告する
	child.lastOutputAt = old
	s.checkOrchestrationChildTimers("s1", time.Now(), cfg)
	if !idleWarned() {
		t.Fatalf("IdleWarned = false, want true (both board and PTY are silent)")
	}

	// PTY 出力が再開 → ラッチ解除（再沈黙時に改めて警告できる）
	child.lastOutputAt = time.Now()
	s.checkOrchestrationChildTimers("s1", time.Now(), cfg)
	if idleWarned() {
		t.Fatalf("IdleWarned = true, want false (activity resumed should clear the latch)")
	}
}

func Test_scanOrchestrationChildFiles(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.ChildTimeoutSeconds = 0
	s.cfg.Orchestration.IdleDoneThresholdSec = 0
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	child.State = "running"
	dir := t.TempDir()
	boardPath := filepath.Join(dir, "board.md")
	if err := os.WriteFile(boardPath, []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-10 * time.Minute)
	s.registerBoardSession("s1", boardPath, parent.ID, "conductor")
	s.registerBoardChild("s1", boardPath, child.ID, parent.ID, child.Role, old)

	childState := func() (lastWrite time.Time, done bool) {
		s.orchestration.mu.Lock()
		defer s.orchestration.mu.Unlock()
		c := s.orchestration.boards["s1"].Children[child.ID]
		return c.LastBoardWrite, c.Done
	}

	// 進捗ファイル未作成 → 変化なし（後方互換: 旧プロンプトの子と同じ扱い）
	s.scanOrchestrationChildFiles("s1", time.Now())
	if lastWrite, _ := childState(); !lastWrite.Equal(old) {
		t.Fatalf("LastBoardWrite changed without progress file")
	}

	// 進捗記帳 → LastBoardWrite 更新・DONE なし
	progressPath := childProgressPath(boardPath, child.ID)
	if err := os.WriteFile(progressPath, []byte("## implementation session=10 2026-07-04T12:00:00+09:00\nstatus: running\nworking\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.scanOrchestrationChildFiles("s1", time.Now())
	lastWrite, done := childState()
	if lastWrite.Equal(old) {
		t.Fatalf("LastBoardWrite not updated after progress write")
	}
	if done {
		t.Fatalf("Done = true, want false (no DONE section yet)")
	}
	if child.State == "done" {
		t.Fatalf("session state = done, want unchanged")
	}

	// DONE 記帳 → Done 検出 + セッション state 遷移
	if err := os.WriteFile(progressPath, []byte("## implementation session=10 2026-07-04T12:00:00+09:00\nstatus: done\n## DONE implementation session=10\nsummary\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.scanOrchestrationChildFiles("s1", time.Now())
	if _, done := childState(); !done {
		t.Fatalf("Done = false, want true after DONE section")
	}
	if child.State != "done" {
		t.Fatalf("session state = %q, want done", child.State)
	}
}

func Test_buildChildInitialPrompt_progressFile(t *testing.T) {
	boardPath := filepath.Join("x", "board.md")
	got := buildChildInitialPrompt("do work", boardPath, "implementation", "orch/x/implementation", 10)
	if !strings.Contains(got, childProgressPath(boardPath, 10)) {
		t.Fatalf("prompt missing progress file path: %s", got)
	}
	if !strings.Contains(got, "Do not write to the shared board") {
		t.Fatalf("prompt missing board write prohibition: %s", got)
	}
	if !strings.Contains(got, "## DONE implementation session=10") {
		t.Fatalf("prompt missing DONE format: %s", got)
	}
}

func Test_appendBoardSection_concurrent(t *testing.T) {
	s := newTestServer()
	path := filepath.Join(t.TempDir(), "board.md")
	const writers = 4
	const perWriter = 100
	var wg sync.WaitGroup
	for i := 0; i < writers; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			for j := 0; j < perWriter; j++ {
				if err := s.appendBoardSection(path, "role", "writer line"); err != nil {
					t.Errorf("append failed: %v", err)
					return
				}
			}
		}(i)
	}
	wg.Wait()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(data), "## role "); got != writers*perWriter {
		t.Fatalf("sections = %d, want %d", got, writers*perWriter)
	}
}

// relay board regression tests (plan_orchestration-relay-loop-c1.md C2): the
// generic DONE / notice / timer handling must step aside for a board that a
// relay owns, and the relay must receive the signal instead.

func Test_scanOrchestrationChildFiles_relayBoardSkipsGenericDone(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	run := h.run(id)
	if err := os.WriteFile(childProgressPath(run.boardPath, impl), []byte(doneLines(relayRoleImplementation, impl, 1, false)), 0o600); err != nil {
		t.Fatal(err)
	}

	h.s.scanOrchestrationChildFiles(id, time.Now())

	if got := h.s.sessions[impl].State; got != "running" {
		t.Fatalf("child state = %q, want running (relay boards never mark done)", got)
	}
	if got := h.status(id); got.State != relayStateReviewing || got.ReviewSessionID == 0 {
		t.Fatalf("relay did not receive the DONE: %+v", got)
	}
	h.s.orchestration.mu.Lock()
	b := h.s.orchestration.boards[id]
	pendingNotices := len(b.PendingNotices)
	child := b.Children[impl]
	fileSize, childDone := child.FileSize, child.Done
	h.s.orchestration.mu.Unlock()
	if pendingNotices != 0 || len(h.s.pendingInput[h.parent.ID]) != 0 {
		t.Fatalf("generic progress notice reached the parent: queued=%d pendingInput=%d", pendingNotices, len(h.s.pendingInput[h.parent.ID]))
	}
	if fileSize == 0 || childDone {
		t.Fatalf("file bookkeeping size=%d done=%v, want size>0 and done=false", fileSize, childDone)
	}
	// A second scan without a file change does nothing.
	before := len(h.injects)
	h.s.scanOrchestrationChildFiles(id, time.Now())
	if len(h.injects) != before {
		t.Fatal("unchanged file triggered another relay transition")
	}
}

func Test_checkOrchestrationChildTimers_relayTimeoutStopsRelay(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	old := time.Now().Add(-10 * time.Minute)
	h.s.orchestration.mu.Lock()
	child := h.s.orchestration.boards[id].Children[impl]
	child.SpawnedAt, child.LastBoardWrite = old, old
	h.s.orchestration.mu.Unlock()

	h.s.checkOrchestrationChildTimers(id, time.Now(), config.OrchestrationConfig{ChildTimeoutSeconds: 1})

	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonTimeout {
		t.Fatalf("relay = %+v, want stopped(timeout)", got)
	}
	if got := h.s.sessions[impl].State; got != "running" {
		t.Fatalf("child state = %q, want running (relay boards never mark timeout)", got)
	}
	if n := len(h.s.pendingInput[h.parent.ID]); n != 0 {
		t.Fatalf("generic ORCHESTRATION-ERROR reached the parent: %d pending inputs", n)
	}
}

func Test_checkOrchestrationChildTimers_relayIdleNudgesChildNotParent(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	old := time.Now().Add(-10 * time.Minute)
	h.s.orchestration.mu.Lock()
	child := h.s.orchestration.boards[id].Children[impl]
	child.SpawnedAt, child.LastBoardWrite = old, old
	h.s.orchestration.mu.Unlock()
	h.s.sessions[impl].lastOutputAt = old
	cfg := config.OrchestrationConfig{IdleDoneThresholdSec: 60}

	h.s.checkOrchestrationChildTimers(id, time.Now(), cfg)

	if len(h.injects) != 1 || h.injects[0].id != impl || !strings.Contains(h.injects[0].text, "This reminder is sent only once") {
		t.Fatalf("injects = %+v, want one nudge to the implementation child", h.injects)
	}
	if n := len(h.s.pendingInput[h.parent.ID]); n != 0 {
		t.Fatalf("generic idle warning reached the parent: %d pending inputs", n)
	}
	data, err := os.ReadFile(h.run(id).boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), fmt.Sprintf("idle role=implementation id=%d", impl)) {
		t.Fatalf("board missing idle record:\n%s", data)
	}
	if got := h.status(id); got.State != relayStateImplementing {
		t.Fatalf("idle changed the relay state: %+v", got)
	}
	// The latch holds while the child stays silent: no second nudge from the timer.
	h.s.checkOrchestrationChildTimers(id, time.Now(), cfg)
	if len(h.injects) != 1 {
		t.Fatalf("injects after second tick = %d, want 1", len(h.injects))
	}
}

func Test_checkOrchestrationChildTimers_terminalRelayStopsWatchingChildren(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	if !h.s.relayFinish(h.run(id), relayStateCompleted, "", "finished by test") {
		t.Fatal("relayFinish returned false")
	}
	old := time.Now().Add(-10 * time.Minute)
	h.s.orchestration.mu.Lock()
	child := h.s.orchestration.boards[id].Children[impl]
	child.SpawnedAt, child.LastBoardWrite = old, old
	h.s.orchestration.mu.Unlock()
	h.s.sessions[impl].lastOutputAt = old
	boardPath := h.run(id).boardPath
	before, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}

	h.s.checkOrchestrationChildTimers(id, time.Now(), config.OrchestrationConfig{IdleDoneThresholdSec: 60})

	after, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(before) || len(h.injects) != 0 {
		t.Fatalf("terminal relay watcher changed state: injects=%d board_changed=%v", len(h.injects), string(after) != string(before))
	}
}

func Test_completeOrchestrationChildOnSessionEnd_relayStopsRelay(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID

	h.s.completeOrchestrationChildOnSessionEnd(impl, "completed")

	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonChildExited {
		t.Fatalf("relay = %+v, want stopped(child_exited)", got)
	}
	if n := len(h.s.pendingInput[h.parent.ID]); n != 0 {
		t.Fatalf("generic session_end notice reached the parent: %d pending inputs", n)
	}
	if len(h.notifies) != 1 || !strings.Contains(h.notifies[0], "relay stopped") || !strings.Contains(h.notifies[0], "child_exited") {
		t.Fatalf("relay finish notice = %q", h.notifies)
	}
	data, err := os.ReadFile(h.run(id).boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "completed without DONE marker") || !strings.Contains(string(data), "relay stopped reason=child_exited") {
		t.Fatalf("board records incomplete:\n%s", data)
	}
}

func boolPtr(v bool) *bool {
	return &v
}

// --- C1 (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md) ---
// childStartupFailed / markChildPromptDelivered / markChildPromptFailed.

// setupStartupFailureBoard registers a parent/child pair on a fresh board and
// returns the child session together with the board ID, for the C1/C2
// startup-failure tests below.
func setupStartupFailureBoard(t *testing.T, s *Server) (parent, child *session, boardID, boardPath string) {
	t.Helper()
	parent = registerTestSession(s, 1, "codex")
	child = registerTestSession(s, 10, "command-code")
	child.ParentSessionID = parent.ID
	child.Role = "tester"
	boardPath = filepath.Join(t.TempDir(), "board.md")
	boardID = "s1"
	s.registerBoardSession(boardID, boardPath, parent.ID, "conductor")
	s.registerBoardChild(boardID, boardPath, child.ID, parent.ID, child.Role, time.Now())
	return parent, child, boardID, boardPath
}

func Test_childStartupFailed_trueWhenDeliveredNoProgressPastGrace(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if !s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = false, want true")
	}
}

func Test_childStartupFailed_falseWhenProgressFileWritten(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	c.FileMod = time.Now().Add(-4 * time.Minute) // 一度でも進捗を書いた
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (child wrote progress; timeout's job, not startup failure)")
	}
}

func Test_childStartupFailed_falseWhenPromptNotConfirmedDelivered(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.StandbySince = time.Now().Add(-2 * time.Minute) // PromptDeliveredAt は未設定のまま
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (delivery never confirmed)")
	}

	// 配送失敗として記録済みの場合も同様に偽（reportInjectFailure が既に報告済み）。
	s.orchestration.mu.Lock()
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.PromptFailed = true
	s.orchestration.mu.Unlock()
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (delivery already reported as failed)")
	}
}

func Test_childStartupFailed_falseWithinGrace(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now() // ちょうど standby に入った直後
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (still within grace)")
	}
}

func Test_childStartupFailed_falseWhileApprovalVisible(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()
	child.approvalVisible = true

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (waiting on an approval prompt, not dead)")
	}
}

func Test_childStartupFailed_disabledByConfig(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60, ChildStartupFail: boolPtr(false)}
	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("childStartupFailed = true, want false (child_startup_fail_enabled: false)")
	}
}

// Test_childStartupFailed_latchPreventsSecondTrue covers the D9 latch itself:
// childStartupFailed reads board.StartupFailed and returns false once it is
// set, even though nothing else about the child changed. Setting the latch is
// handleChildStartupFailed's job (C2); this only tests the read side.
func Test_childStartupFailed_latchPreventsSecondTrue(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}
	if !s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("first call: childStartupFailed = false, want true")
	}

	// handleChildStartupFailed が実際に立てるラッチを模して立てる。
	s.orchestration.mu.Lock()
	s.orchestration.boards[boardID].StartupFailed[child.ID] = true
	s.orchestration.mu.Unlock()

	if s.childStartupFailed(child.ID, time.Now(), cfg) {
		t.Fatal("second call after latch: childStartupFailed = true, want false")
	}
}

// TestInjectInitialPromptNotify_RecordsPromptDelivered exercises the
// "echo observed" success exit of injectInitialPromptNotify and confirms it
// records PromptDeliveredAt (and leaves PromptFailed false). Follows the same
// fake-wrapper-echoes-back pattern as
// TestInjectInitialPromptNotify_ReadySignalInjectsOnce (orchestration_composer_guard_test.go).
func TestInjectInitialPromptNotify_RecordsPromptDelivered(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 2, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "tester"
	child.vt = newVTBuffer(80, 24)
	child.vt.Write([]byte("Ask Codex to do anything\r\n? for shortcuts\r\n"))
	boardPath := filepath.Join(t.TempDir(), "board.md")
	s.registerBoardSession("s1", boardPath, parent.ID, "conductor")
	s.registerBoardChild("s1", boardPath, child.ID, parent.ID, child.Role, time.Now())

	s.sessionsMu.Lock()
	s.wrappers[2] = &wrapperConn{sendFunc: func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok || msg.Type != "pty_input" {
			return nil
		}
		data := string(msg.Data)
		s.sessionsMu.Lock()
		if cur := s.sessions[2]; cur != nil && cur.vt != nil {
			cur.vt.Write(msg.Data)
		}
		s.sessionsMu.Unlock()
		if data == "\r" {
			go func() {
				time.Sleep(5 * time.Millisecond)
				s.sessionsMu.Lock()
				if cur := s.sessions[2]; cur != nil {
					cur.lastOutputAt = time.Now()
				}
				s.sessionsMu.Unlock()
			}()
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	before := time.Now()
	s.injectInitialPromptNotify(2, "hello", injectNotice{ParentID: 1, BoardPath: boardPath, Role: "tester"})

	s.orchestration.mu.Lock()
	c := s.orchestration.boards["s1"].Children[2]
	delivered, failed := c.PromptDeliveredAt, c.PromptFailed
	s.orchestration.mu.Unlock()
	if delivered.IsZero() || delivered.Before(before) {
		t.Fatalf("PromptDeliveredAt = %v, want a time at/after %v", delivered, before)
	}
	if failed {
		t.Fatal("PromptFailed = true, want false on a successful delivery")
	}
}

// TestInjectInitialPromptNotify_RecordsPromptFailedOnBlockedModal exercises
// the composerBlocked exit and confirms it records PromptFailed (and leaves
// PromptDeliveredAt zero). Same setup as
// TestInjectInitialPromptNotify_BlockedModalNeverInjects (orchestration_composer_guard_test.go),
// which already proves this returns quickly instead of blocking for
// orchestrationComposerMaxWait.
func TestInjectInitialPromptNotify_RecordsPromptFailedOnBlockedModal(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 2, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "tester"
	child.vt = newVTBuffer(80, 24)
	child.vt.Write([]byte("Do you trust the contents of this directory?\r\nPress enter to continue\r\n"))
	boardPath := filepath.Join(t.TempDir(), "board.md")
	s.registerBoardSession("s1", boardPath, parent.ID, "conductor")
	s.registerBoardChild("s1", boardPath, child.ID, parent.ID, child.Role, time.Now())

	notice := injectNotice{ParentID: 1, BoardPath: boardPath, Role: "tester"}
	s.injectInitialPromptNotify(2, "some prompt", notice)

	s.orchestration.mu.Lock()
	c := s.orchestration.boards["s1"].Children[2]
	delivered, failed := c.PromptDeliveredAt, c.PromptFailed
	s.orchestration.mu.Unlock()
	if !failed {
		t.Fatal("PromptFailed = false, want true after composerBlocked")
	}
	if !delivered.IsZero() {
		t.Fatalf("PromptDeliveredAt = %v, want zero (never delivered)", delivered)
	}
}

// TestSanitizeInjectText_StripsControlBytes covers the D4 sanitization step
// handleChildStartupFailed applies to the captured screen tail before it
// reaches the board or the parent notification. The vt buffer itself already
// drops bare control bytes below 0x20 when writing cells (vt_buffer.go
// writeRune's `if r < 0x20 { return }`), so a synthetic vt screen can never
// actually contain them — this tests the sanitizer function directly, which
// is the actual mechanism relied upon if that ever changes.
func TestSanitizeInjectText_StripsControlBytes(t *testing.T) {
	raw := "line one\x1b[31mred\x07 bell\x7fdel"
	got := sanitizeInjectText(raw)
	if strings.ContainsAny(got, "\x1b\x07\x7f") {
		t.Fatalf("sanitizeInjectText left control bytes: %q", got)
	}
	for _, want := range []string{"line one", "red", "bell", "del"} {
		if !strings.Contains(got, want) {
			t.Fatalf("sanitizeInjectText removed too much (%q): %q", want, got)
		}
	}
}

// --- C2 (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md) ---
// checkOrchestrationChildTimers' startup_failed branch / handleChildStartupFailed.

func Test_checkOrchestrationChildTimers_startupFailedNotifiesRecordsAndStops(t *testing.T) {
	s := newTestServer()
	parent, child, boardID, boardPath := setupStartupFailureBoard(t, s)
	child.vt = newVTBuffer(80, 24)
	child.vt.Write([]byte("Error: 403 The free MiniMax M3 model has been retired.\r\n"))

	var dismissed []proto.Message
	s.sessionsMu.Lock()
	s.wrappers[child.ID] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok {
			dismissed = append(dismissed, msg)
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 1}
	s.checkOrchestrationChildTimers(boardID, time.Now(), cfg)

	// board 記帳: child-<id>.md ではなく board 本体へ "hub" 名義で。
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	board := string(data)
	for _, want := range []string{"## hub ", "startup failed", "role=tester id=10", "403"} {
		if !strings.Contains(board, want) {
			t.Errorf("board missing %q:\n%s", want, board)
		}
	}
	if _, err := os.Stat(childProgressPath(boardPath, child.ID)); err == nil {
		t.Error("startup failure record leaked into child-<id>.md; it must go to the board body")
	}

	// 親への通知。
	s.sessionsMu.Lock()
	pending := append([]string(nil), s.pendingInput[parent.ID]...)
	s.sessionsMu.Unlock()
	joined := strings.Join(pending, "")
	for _, want := range []string{"[MANY-AI-CLI-ORCHESTRATION-ERROR]", "limit=startup_failed", "role=tester id=10", "403"} {
		if !strings.Contains(joined, want) {
			t.Errorf("parent notification missing %q: %q", want, joined)
		}
	}

	// セッション終端化 (D8)。
	if child.State != "error" {
		t.Fatalf("child.State = %q, want error", child.State)
	}

	// 停止 (D6, C3): 既定 ON なので killWrapper が呼ばれる。
	if len(dismissed) != 1 {
		t.Fatalf("dismissed messages = %d, want 1", len(dismissed))
	}
	if dismissed[0].Type != proto.TypeSessionDismissed || dismissed[0].SessionID != child.ID || dismissed[0].Reason != "startup_failed" {
		t.Fatalf("dismiss message = %+v, want session_dismissed/startup_failed for session %d", dismissed[0], child.ID)
	}

	// ラッチ (D9)。
	s.orchestration.mu.Lock()
	latched := s.orchestration.boards[boardID].StartupFailed[child.ID]
	s.orchestration.mu.Unlock()
	if !latched {
		t.Fatal("StartupFailed latch not set")
	}

	// respawn 抑止 (D5): 子が増えていない。
	s.orchestration.mu.Lock()
	childCount := len(s.orchestration.boards[boardID].Children)
	s.orchestration.mu.Unlock()
	if childCount != 1 {
		t.Fatalf("children after startup failure = %d, want 1 (respawnTimedOutChild must not run)", childCount)
	}

	// 二重通知抑止 (D9): timeout 閾値を越えても回した 2 回目で増えない。
	s.orchestration.mu.Lock()
	c.SpawnedAt = time.Now().Add(-2 * time.Second)
	s.orchestration.mu.Unlock()
	s.checkOrchestrationChildTimers(boardID, time.Now(), config.OrchestrationConfig{ChildTimeoutSeconds: 1})

	s.sessionsMu.Lock()
	pending2 := append([]string(nil), s.pendingInput[parent.ID]...)
	s.sessionsMu.Unlock()
	if len(pending2) != len(pending) {
		t.Fatalf("second tick added parent notifications: before=%d after=%d", len(pending), len(pending2))
	}
	if len(dismissed) != 1 {
		t.Fatalf("second tick re-killed the wrapper: %d dismiss messages, want 1", len(dismissed))
	}
	s.orchestration.mu.Lock()
	timedOut := s.orchestration.boards[boardID].TimedOut[child.ID]
	s.orchestration.mu.Unlock()
	if timedOut {
		t.Fatal("second tick marked TimedOut despite the StartupFailed latch")
	}
}

// Test_handleChildStartupFailed_stopsAfterBoardAndNotify is a genuinely
// order-sensitive check (not just a final-state check): the fake wrapper's
// sendFunc fires exactly when killWrapper sends the dismiss message
// (synchronously, same goroutine), so if the board record or the parent
// notification had not been written by that point, this fails. A
// reordering that moved the kill before the board record / notify would
// make both booleans below false.
func Test_handleChildStartupFailed_stopsAfterBoardAndNotify(t *testing.T) {
	s := newTestServer()
	parent, child, boardID, boardPath := setupStartupFailureBoard(t, s)

	var killed, boardHadRecordAtKillTime, parentHadNoticeAtKillTime bool
	s.sessionsMu.Lock()
	s.wrappers[child.ID] = &wrapperConn{sendFunc: func(m any) error {
		killed = true
		if data, err := os.ReadFile(boardPath); err == nil {
			boardHadRecordAtKillTime = strings.Contains(string(data), "startup failed")
		}
		s.sessionsMu.Lock()
		joined := strings.Join(s.pendingInput[parent.ID], "")
		s.sessionsMu.Unlock()
		parentHadNoticeAtKillTime = strings.Contains(joined, "[MANY-AI-CLI-ORCHESTRATION-ERROR]")
		return nil
	}}
	s.sessionsMu.Unlock()

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	s.checkOrchestrationChildTimers(boardID, time.Now(), config.OrchestrationConfig{ChildStartupGraceSeconds: 1})

	if !killed {
		t.Fatal("wrapper never received a dismiss message")
	}
	if !boardHadRecordAtKillTime {
		t.Fatal("board record was not present yet when the wrapper was stopped (kill ran before the board record)")
	}
	if !parentHadNoticeAtKillTime {
		t.Fatal("parent notification was not present yet when the wrapper was stopped (kill ran before the notify)")
	}
}

func Test_checkOrchestrationChildTimers_startupFailedSkipsKillWhenDisabled(t *testing.T) {
	s := newTestServer()
	_, child, boardID, _ := setupStartupFailureBoard(t, s)

	var dismissed []proto.Message
	s.sessionsMu.Lock()
	s.wrappers[child.ID] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok {
			dismissed = append(dismissed, msg)
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	s.orchestration.mu.Lock()
	c := s.orchestration.boards[boardID].Children[child.ID]
	c.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	c.StandbySince = time.Now().Add(-2 * time.Minute)
	s.orchestration.mu.Unlock()

	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 1, ChildStartupKill: boolPtr(false)}
	s.checkOrchestrationChildTimers(boardID, time.Now(), cfg)

	if len(dismissed) != 0 {
		t.Fatalf("dismissed messages = %d, want 0 (child_startup_kill: false)", len(dismissed))
	}
	// 通知とラッチは child_startup_kill と独立に起きる。
	if child.State != "error" {
		t.Fatalf("child.State = %q, want error (notify still happens when only the kill is disabled)", child.State)
	}
}

func Test_checkOrchestrationChildTimers_relayBoardRoutesStartupFailedToRelay(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID

	h.s.orchestration.mu.Lock()
	child := h.s.orchestration.boards[id].Children[impl]
	child.PromptDeliveredAt = time.Now().Add(-5 * time.Minute)
	child.StandbySince = time.Now().Add(-2 * time.Minute)
	h.s.orchestration.mu.Unlock()

	// ChildStartupKill を無効化する: relayHarness の fake spawn は
	// newWrapperConn(&websocket.Conn{}) という「実 API を持つが未接続」の wrapper を
	// 登録しており、killWrapper の close() を実際に走らせるとゼロ値 Conn への
	// メソッド呼び出しで panic する。この test の関心は relay 振り分けであって
	// kill 機構そのものではない（kill は Test_checkOrchestrationChildTimers_startupFailedNotifiesRecordsAndStops
	// が別途カバーする）。
	h.s.checkOrchestrationChildTimers(id, time.Now(), config.OrchestrationConfig{ChildStartupGraceSeconds: 1, ChildStartupKill: boolPtr(false)})

	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonStartupFailed {
		t.Fatalf("relay = %+v, want stopped(startup_failed)", got)
	}
	if n := len(h.s.pendingInput[h.parent.ID]); n != 0 {
		t.Fatalf("generic ORCHESTRATION-ERROR reached the parent: %d pending inputs", n)
	}
	if got := h.s.sessions[impl].State; got != "running" {
		t.Fatalf("child session state = %q, want unchanged (relay boards never call markChildState, same as timeout)", got)
	}
}

// --- C1 (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md):
// spawn confirmations are a Hub-side hold decoupled from the HTTP request
// that registered them, with no deadline, and only a human's explicit
// refusal may ever write user_refusal to the board. The four tests below
// pin exactly that.

// Test_handleSpawnChild_callerContextCanceled_pendingSurvives は、呼び出し元の
// HTTP リクエストが（相手 AI のシェルツール timeout 等で）先に死んでも、保留が
// map から消えず、応答も書かれないことを確認する。
func Test_handleSpawnChild_callerContextCanceled_pendingSurvives(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Orchestration.SpawnConfirmMode = config.SpawnConfirmOn
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()

	ctx, cancel := context.WithCancel(context.Background())
	cancel() // 呼び出し元がすでに諦めている状態を再現する

	req := orchestrationRequest(http.MethodPost, "/api/sessions/1/spawn-child", spawnChildRequest{Provider: "codex", Role: "tester", InitialPrompt: "hi"})
	req = req.WithContext(ctx)
	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, req)

	if rr.Body.Len() != 0 {
		t.Fatalf("handler must not write a response when the caller's context is already done, got status=%d body=%q", rr.Code, rr.Body.String())
	}

	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	if len(s.orchestration.spawnConfirmations) != 1 {
		t.Fatalf("spawnConfirmations = %d entries, want 1 (must survive the dead caller)", len(s.orchestration.spawnConfirmations))
	}
	for _, p := range s.orchestration.spawnConfirmations {
		if p.Decided {
			t.Fatal("a confirmation must not be marked decided just because its waiter left")
		}
		if !p.WaiterGone {
			t.Fatal("waitSpawnConfirmation must record that the waiter is gone")
		}
	}
}

// Test_resolveSpawnConfirmationDecision_refusalWritesUserRefusalOnce は、人が
// 明示的に拒否したときだけ board へ user_refusal が書かれることを確認する。
func Test_resolveSpawnConfirmationDecision_refusalWritesUserRefusalOnce(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()

	pending := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex", InitialPrompt: "hi"})
	s.resolveSpawnConfirmationDecision(pending, false, "", "")

	select {
	case outcome := <-pending.Outcome:
		if outcome.Approved {
			t.Fatal("a refusal outcome must not be Approved")
		}
	default:
		t.Fatal("a decided refusal must write an outcome")
	}

	if parent.OrchestrationID == "" {
		t.Fatal("a human refusal must create a board for the parent")
	}
	dir, err := config.Dir()
	if err != nil {
		t.Fatalf("config.Dir: %v", err)
	}
	boardPath := filepath.Join(dir, "orchestration", safeToken(parent.OrchestrationID), "board.md")
	board, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatalf("read board: %v", err)
	}
	if !strings.Contains(string(board), "refused: role=tester reason=user_refusal") {
		t.Fatalf("board missing the user_refusal line:\n%s", board)
	}
}

// Test_registerSpawnConfirmation_supersedesSameParentRole は、同じ親・同じ role の
// 要求が 2 回来たら保留が 1 件になり、古い方が superseded で閉じる（user_refusal を
// 書かない）ことを確認する。
func Test_registerSpawnConfirmation_supersedesSameParentRole(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()

	first := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex"})
	second := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex"})

	select {
	case outcome := <-first.Outcome:
		if outcome.Approved {
			t.Fatal("a superseded confirmation must not be Approved")
		}
	default:
		t.Fatal("registering a second confirmation for the same parent+role must close the older one")
	}
	if !first.Decided {
		t.Fatal("the superseded confirmation must be marked decided")
	}

	s.orchestration.mu.Lock()
	_, firstStillPending := s.orchestration.spawnConfirmations[first.ID]
	_, secondPending := s.orchestration.spawnConfirmations[second.ID]
	count := len(s.orchestration.spawnConfirmations)
	s.orchestration.mu.Unlock()
	if firstStillPending {
		t.Fatal("the superseded confirmation must be removed from the pending map")
	}
	if !secondPending {
		t.Fatal("the newer confirmation must remain pending")
	}
	if count != 1 {
		t.Fatalf("pending confirmations = %d, want 1 (one per parent+role)", count)
	}

	dir, err := config.Dir()
	if err != nil {
		t.Fatalf("config.Dir: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "orchestration")); !os.IsNotExist(err) {
		t.Fatalf("supersede must never write a board (that would only happen on a human refusal), stat err=%v", err)
	}
}

// Test_expireSpawnConfirmationsForParent_withoutUserRefusal は
// expireSpawnConfirmationsForParent 自体（parentID が本当に消えたときの失効経路）が、
// parent_gone で失効し user_refusal を書かないことを確認する。
//
// C5 (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md) より前は
// handleDismiss がこの関数を無条件に呼んでいたため、このテストは handleDismiss 経由で
// 書かれていた。C5 で handleDismiss は保留があるセッションの dismiss 自体を拒否するよう
// になった（Test_handleDismiss_refusesDismissWhilePendingSpawnConfirmation 参照）ので、
// handleDismiss はもうこのケースでこの関数へ到達しない（到達するのは、チェック後・削除前
// の狭い競合窓で登録された保留だけ）。そのため本テストは expireSpawnConfirmationsForParent
// を直接呼び、失効そのものの挙動を引き続き固定する。
func Test_expireSpawnConfirmationsForParent_withoutUserRefusal(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()

	pending := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex"})

	s.expireSpawnConfirmationsForParent(parent.ID, "dismiss")

	select {
	case outcome := <-pending.Outcome:
		if outcome.Approved {
			t.Fatal("a parent_gone outcome must not be Approved")
		}
	default:
		t.Fatal("expiring the parent's confirmations must write an outcome")
	}
	if !pending.Decided {
		t.Fatal("the expired confirmation must be marked decided")
	}

	s.orchestration.mu.Lock()
	_, stillPending := s.orchestration.spawnConfirmations[pending.ID]
	s.orchestration.mu.Unlock()
	if stillPending {
		t.Fatal("the expired confirmation must be removed from the pending map")
	}

	dir, err := config.Dir()
	if err != nil {
		t.Fatalf("config.Dir: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "orchestration")); !os.IsNotExist(err) {
		t.Fatalf("the parent going away must never write a board (that would only happen on a human refusal), stat err=%v", err)
	}
}

// Test_handleDismiss_refusesDismissWhilePendingSpawnConfirmation は C5 の完了条件
// そのもの: 保留を持つ親セッションに対する handleDismiss がセッションを消さず、保留も
// 決定せずに残し、session_dismiss_refused を broadcast することを確認する。
func Test_handleDismiss_refusesDismissWhilePendingSpawnConfirmation(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()
	wc := &wrapperConn{}
	s.sessionsMu.Lock()
	s.wrappers[parent.ID] = wc
	s.sessionsMu.Unlock()

	pending := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex"})

	ui := registerTestUI(s)
	events := make(chan proto.Message, 8)
	s.sessionsMu.Lock()
	s.uis[ui].sendFunc = func(m any) error {
		if msg, ok := m.(proto.Message); ok {
			events <- msg
		}
		return nil
	}
	s.sessionsMu.Unlock()

	skip := s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: parent.ID})
	if !skip {
		t.Fatal("a refused dismiss must report skip=true, like the existing idempotent-dismiss case")
	}

	s.sessionsMu.Lock()
	_, sessionStillExists := s.sessions[parent.ID]
	_, wrapperStillExists := s.wrappers[parent.ID]
	s.sessionsMu.Unlock()
	if !sessionStillExists {
		t.Fatal("a session holding a pending spawn confirmation must not be removed by dismiss")
	}
	if !wrapperStillExists {
		t.Fatal("the wrapper must not be torn down when the dismiss is refused")
	}

	select {
	case outcome := <-pending.Outcome:
		t.Fatalf("a refused dismiss must not decide the pending confirmation, got outcome=%+v", outcome)
	default:
	}
	if pending.Decided {
		t.Fatal("a refused dismiss must leave the pending confirmation undecided")
	}
	s.orchestration.mu.Lock()
	_, stillPending := s.orchestration.spawnConfirmations[pending.ID]
	s.orchestration.mu.Unlock()
	if !stillPending {
		t.Fatal("a refused dismiss must leave the confirmation in the pending map")
	}

	var sawRefusal bool
	drain := true
	for drain {
		select {
		case msg := <-events:
			if msg.Type == "session_dismiss_refused" && msg.SessionID == parent.ID {
				sawRefusal = true
			}
		default:
			drain = false
		}
	}
	if !sawRefusal {
		t.Fatal("a refused dismiss must broadcast session_dismiss_refused for the parent session")
	}

	dir, err := config.Dir()
	if err != nil {
		t.Fatalf("config.Dir: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, "orchestration")); !os.IsNotExist(err) {
		t.Fatalf("refusing a dismiss must never write a board, stat err=%v", err)
	}
}

// Test_hasPendingSpawnConfirmation exercises the guard predicate directly:
// false with nothing registered, true while a confirmation is undecided,
// false again once it has been decided.
func Test_hasPendingSpawnConfirmation(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")

	if s.hasPendingSpawnConfirmation(parent.ID) {
		t.Fatal("want false before any confirmation is registered")
	}

	pending := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "codex"})
	if !s.hasPendingSpawnConfirmation(parent.ID) {
		t.Fatal("want true while the confirmation is undecided")
	}
	if s.hasPendingSpawnConfirmation(parent.ID + 1) {
		t.Fatal("want false for an unrelated session id")
	}

	s.orchestration.mu.Lock()
	pending.Decided = true
	delete(s.orchestration.spawnConfirmations, pending.ID)
	s.orchestration.mu.Unlock()

	if s.hasPendingSpawnConfirmation(parent.ID) {
		t.Fatal("want false once the confirmation has been decided and removed")
	}
}

package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/config"
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

func Test_handleSendChild(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	parent := registerTestSession(s, 1, "codex")
	child := registerTestSession(s, 10, "codex")
	child.ParentSessionID = parent.ID
	child.Role = "implementation"
	child.State = "running"
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

func boolPtr(v bool) *bool {
	return &v
}

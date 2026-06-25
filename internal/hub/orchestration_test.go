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

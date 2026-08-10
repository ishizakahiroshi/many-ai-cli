package hub

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

func TestHandleAgentChatReadsClaudeTranscript(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	sid := "123e4567-e89b-42d3-a456-426614174000"
	cwd := `C:\workspace\many-ai-cli`
	claudeDir := t.TempDir()
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	writeClaudeTranscript(t, projectDir, sid+".jsonl", cwd, "2026-07-20T11:12:25Z")
	ses := registerTestSession(s, 1, "claude")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = "2026-07-20T11:12:24Z"
	ses.ClaudeDir = claudeDir
	ses.AgentSessionID = sid
	s.sessionsMu.Unlock()

	req := httptest.NewRequest(http.MethodGet, "/api/agent-chat?token=tok&session_id=1&limit=10", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	w := httptest.NewRecorder()
	s.handleAgentChat(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var response struct {
		OK         bool               `json:"ok"`
		Available  bool               `json:"available"`
		Total      int                `json:"total"`
		TotalKnown bool               `json:"total_known"`
		Messages   []agentChatMessage `json:"messages"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if !response.OK || !response.Available || response.Total != -1 || response.TotalKnown || len(response.Messages) != 1 {
		t.Fatalf("unexpected response: %#v", response)
	}
}

func TestHandleAgentChatMarksUnsupportedProviderUnavailable(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	registerTestSession(s, 1, "copilot")
	req := httptest.NewRequest(http.MethodGet, "/api/agent-chat?token=tok&session_id=1", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	w := httptest.NewRecorder()
	s.handleAgentChat(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var response struct {
		Available bool `json:"available"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if response.Available {
		t.Fatal("unsupported provider should report available=false")
	}
}

func TestAgentChatReattachDropsStalePollAndDoesNotShareState(t *testing.T) {
	oldState := newAgentChatParseState()
	old := &session{
		ID:                  1,
		Provider:            "claude",
		agentChatRunning:    true,
		agentChatGeneration: 7,
		agentChatParseState: oldState,
	}
	s := &Server{sessions: map[int]*session{1: old}}
	s.sessionsMu.Lock()
	s.stopAgentChatTailLocked(old)
	replacement := &session{
		ID:                  1,
		Provider:            "claude",
		agentChatRunning:    true,
		agentChatGeneration: 1,
		agentChatParseState: newAgentChatParseState(),
	}
	s.sessions[1] = replacement
	s.sessionsMu.Unlock()

	if replacement.agentChatParseState == oldState {
		t.Fatal("reattach shared mutable agent-chat parse state")
	}
	if s.broadcastAgentChatIfCurrent(1, 7, proto.Message{Type: "agent_chat", SessionID: 1}) {
		t.Fatal("stale poll was allowed to broadcast after reattach")
	}
}

func TestPollAgentChatPrimeKeepsSafeBoundaryUntilTailCompletes(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	sid := "123e4567-e89b-42d3-a456-426614174000"
	cwd := `C:\workspace\many-ai-cli`
	claudeDir := t.TempDir()
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	if err := os.MkdirAll(projectDir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(projectDir, sid+".jsonl")
	complete := `{"type":"user","message":{"role":"user","content":"before-prime"}}` + "\n"
	partial := `{"type":"user","message":{"role":"user","content":"after-prime"}`
	if err := os.WriteFile(path, []byte(complete+partial), 0o600); err != nil {
		t.Fatal(err)
	}
	ses := registerTestSession(s, 1, "claude")
	now := time.Now().UTC().Format(time.RFC3339)
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = now
	ses.ClaudeDir = claudeDir
	ses.AgentSessionID = sid
	ses.agentChatRunning = true
	ses.agentChatGeneration = 1
	s.sessionsMu.Unlock()

	s.pollAgentChat(1, 1)
	s.sessionsMu.Lock()
	primedOffset := ses.agentChatOffset
	if ses.agentChatTimer != nil {
		ses.agentChatTimer.Stop()
		ses.agentChatTimer = nil
	}
	s.sessionsMu.Unlock()
	if primedOffset != int64(len(complete)) {
		t.Fatalf("live prime advanced past incomplete tail: got=%d want=%d", primedOffset, len(complete))
	}

	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString("}\n"); err != nil {
		_ = f.Close()
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	s.pollAgentChat(1, 1)
	s.sessionsMu.Lock()
	finalOffset := ses.agentChatOffset
	state := ses.agentChatParseState
	s.stopAgentChatTailLocked(ses)
	s.sessionsMu.Unlock()
	stat, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if finalOffset != stat.Size() || state == nil {
		t.Fatalf("completed tail was not recovered after prime: offset=%d size=%d state=%+v", finalOffset, stat.Size(), state)
	}
}

func TestPollAgentChatPrimeRetriesUncommittedTailPageForClaudeAndCodex(t *testing.T) {
	tests := []struct {
		name  string
		line  func(string) string
		setup func(*session, string, string)
	}{
		{
			name: "claude",
			line: func(text string) string {
				return fmt.Sprintf(`{"type":"user","message":{"role":"user","content":"%s"}}`, text)
			},
			setup: func(ses *session, claudeDir, _ string) {
				ses.ClaudeDir = claudeDir
				ses.AgentSessionID = "123e4567-e89b-42d3-a456-426614174000"
			},
		},
		{
			name: "codex",
			line: func(text string) string {
				return fmt.Sprintf(`{"type":"event_msg","payload":{"type":"user_message","message":"%s"}}`, text)
			},
			setup: func(ses *session, _, path string) {
				ses.CodexHome = filepath.Dir(path)
				ses.NativeLogPath = path
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			s := newTestServer()
			sid := "123e4567-e89b-42d3-a456-426614174000"
			cwd := `C:\workspace\many-ai-cli`
			var path, claudeDir string
			if tt.name == "claude" {
				claudeDir = t.TempDir()
				projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
				if err := os.MkdirAll(projectDir, 0o700); err != nil {
					t.Fatal(err)
				}
				path = filepath.Join(projectDir, sid+".jsonl")
			} else {
				path = filepath.Join(t.TempDir(), "rollout.jsonl")
			}
			var data strings.Builder
			for i := 0; i < 3; i++ {
				data.WriteString(tt.line(fmt.Sprintf("prime-%d", i)))
				data.WriteByte('\n')
			}
			if err := os.WriteFile(path, []byte(data.String()), 0o600); err != nil {
				t.Fatal(err)
			}

			ses := registerTestSession(s, 1, tt.name)
			now := time.Now().UTC().Format(time.RFC3339)
			s.sessionsMu.Lock()
			ses.CWD = cwd
			ses.StartedAt = now
			ses.agentChatRunning = true
			ses.agentChatGeneration = 1
			tt.setup(ses, claudeDir, path)
			s.sessionsMu.Unlock()

			base := time.Unix(1_700_000_000, 0)
			clockCalls := 0
			s.agentChatReadClock = func() time.Time {
				clockCalls++
				if clockCalls >= 3 {
					return base.Add(time.Second)
				}
				return base
			}
			s.pollAgentChat(1, 1)
			s.sessionsMu.Lock()
			firstOffset := ses.agentChatOffset
			firstState := ses.agentChatParseState
			if ses.agentChatTimer != nil {
				ses.agentChatTimer.Stop()
				ses.agentChatTimer = nil
			}
			s.sessionsMu.Unlock()
			if firstOffset != 0 || firstState != nil {
				t.Fatalf("uncommitted prime advanced live state: offset=%d state=%+v calls=%d", firstOffset, firstState, clockCalls)
			}

			s.agentChatReadClock = func() time.Time { return base }
			s.pollAgentChat(1, 1)
			s.sessionsMu.Lock()
			finalOffset := ses.agentChatOffset
			finalState := ses.agentChatParseState
			if ses.agentChatTimer != nil {
				ses.agentChatTimer.Stop()
				ses.agentChatTimer = nil
			}
			s.stopAgentChatTailLocked(ses)
			s.sessionsMu.Unlock()
			stat, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			if finalOffset != stat.Size() || finalState == nil || !finalState.lastRead.DecodeCommitted || len(finalState.messages) != 3 {
				t.Fatalf("retry did not commit complete tail page: offset=%d size=%d state=%+v", finalOffset, stat.Size(), finalState)
			}
		})
	}
}

func TestHandleAgentChatUsesBoundedBackwardCursorPages(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	sid := "123e4567-e89b-42d3-a456-426614174000"
	cwd := `C:\workspace\many-ai-cli`
	claudeDir := t.TempDir()
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	if err := os.MkdirAll(projectDir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(projectDir, sid+".jsonl")
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 600; i++ {
		if _, err := fmt.Fprintf(f, `{"type":"user","message":{"role":"user","content":"message-%03d"}}`+"\n", i); err != nil {
			_ = f.Close()
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	ses := registerTestSession(s, 1, "claude")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = "2026-07-20T11:12:24Z"
	ses.ClaudeDir = claudeDir
	ses.AgentSessionID = sid
	s.sessionsMu.Unlock()

	request := func(query string) struct {
		Total      int                `json:"total"`
		TotalKnown bool               `json:"total_known"`
		Offset     int64              `json:"offset"`
		NextCursor int64              `json:"next_cursor"`
		HasMore    bool               `json:"has_more"`
		Messages   []agentChatMessage `json:"messages"`
	} {
		req := httptest.NewRequest(http.MethodGet, "/api/agent-chat?token=tok&session_id=1&limit=50&"+query, nil)
		req.Host = "127.0.0.1:47777"
		req.RemoteAddr = "127.0.0.1:12345"
		w := httptest.NewRecorder()
		s.handleAgentChat(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
		}
		var response struct {
			Total      int                `json:"total"`
			TotalKnown bool               `json:"total_known"`
			Offset     int64              `json:"offset"`
			NextCursor int64              `json:"next_cursor"`
			HasMore    bool               `json:"has_more"`
			Messages   []agentChatMessage `json:"messages"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
			t.Fatal(err)
		}
		return response
	}
	first := request("")
	if first.Total != -1 || first.TotalKnown || len(first.Messages) != 50 || !first.HasMore || first.NextCursor != first.Offset {
		t.Fatalf("initial tail page is not bounded/cursorized: %+v", first)
	}
	if !strings.HasPrefix(first.Messages[len(first.Messages)-1].Text, "message-599") {
		t.Fatalf("initial page did not return the newest messages: %#v", first.Messages[len(first.Messages)-1])
	}
	second := request("cursor=" + strconv.FormatInt(first.NextCursor, 10))
	if len(second.Messages) == 0 || second.Messages[len(second.Messages)-1].Text == first.Messages[0].Text || second.NextCursor >= first.NextCursor {
		t.Fatalf("backward cursor page overlapped or did not move backward: first=%+v second=%+v", first, second)
	}
}

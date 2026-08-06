package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
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
		OK        bool               `json:"ok"`
		Available bool               `json:"available"`
		Total     int                `json:"total"`
		Messages  []agentChatMessage `json:"messages"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if !response.OK || !response.Available || response.Total != 1 || len(response.Messages) != 1 {
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

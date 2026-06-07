package hub

import (
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func newSessionLogTestServer(t *testing.T) (*Server, string) {
	t.Helper()
	root := t.TempDir()
	s := newSecTestServer(t, root)
	s.cfg.Hub.LogDir = root
	sessionsDir := filepath.Join(root, "sessions")
	if err := os.MkdirAll(sessionsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	return s, sessionsDir
}

func sessionLogGet(t *testing.T, s *Server, url string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, url, nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleSessionLog(w, req)
	return w
}

type sessionLogResp struct {
	OK      bool   `json:"ok"`
	Size    int64  `json:"size"`
	Offset  int64  `json:"offset"`
	Length  int    `json:"length"`
	DataB64 string `json:"data_b64"`
}

func decodeSessionLogResp(t *testing.T, w *httptest.ResponseRecorder) sessionLogResp {
	t.Helper()
	var resp sessionLogResp
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("invalid json: %v: %s", err, w.Body.String())
	}
	return resp
}

func TestHandleSessionLogRequiresToken(t *testing.T) {
	s, _ := newSessionLogTestServer(t)
	w := sessionLogGet(t, s, "/api/session-log?session_id=1")
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

func TestHandleSessionLogUnknownSession(t *testing.T) {
	s, _ := newSessionLogTestServer(t)
	w := sessionLogGet(t, s, "/api/session-log?token=tok&session_id=99")
	if w.Code != http.StatusNotFound {
		t.Fatalf("expected 404, got %d: %s", w.Code, w.Body.String())
	}
}

func TestHandleSessionLogInvalidSessionID(t *testing.T) {
	s, _ := newSessionLogTestServer(t)
	w := sessionLogGet(t, s, "/api/session-log?token=tok&session_id=abc")
	if w.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d", w.Code)
	}
}

func TestHandleSessionLogTailAndRange(t *testing.T) {
	s, sessionsDir := newSessionLogTestServer(t)
	logPath := filepath.Join(sessionsDir, "claude_x_s1.log")
	content := []byte("0123456789abcdefghij") // 20 bytes
	if err := os.WriteFile(logPath, content, 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[1] = &session{ID: 1, LogPath: logPath}

	// offset 省略 → 末尾から limit バイト
	w := sessionLogGet(t, s, "/api/session-log?token=tok&session_id=1&limit=8")
	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", w.Code, w.Body.String())
	}
	resp := decodeSessionLogResp(t, w)
	if resp.Size != 20 || resp.Offset != 12 {
		t.Fatalf("unexpected tail range: %+v", resp)
	}
	data, err := base64.StdEncoding.DecodeString(resp.DataB64)
	if err != nil || string(data) != "cdefghij" {
		t.Fatalf("unexpected tail data: %q err=%v", data, err)
	}

	// 範囲指定
	w = sessionLogGet(t, s, "/api/session-log?token=tok&session_id=1&offset=0&limit=10")
	resp = decodeSessionLogResp(t, w)
	data, _ = base64.StdEncoding.DecodeString(resp.DataB64)
	if resp.Offset != 0 || string(data) != "0123456789" {
		t.Fatalf("unexpected range data: %q resp=%+v", data, resp)
	}

	// offset がサイズ超過 → 空データ
	w = sessionLogGet(t, s, "/api/session-log?token=tok&session_id=1&offset=100&limit=10")
	resp = decodeSessionLogResp(t, w)
	if resp.Offset != 20 || resp.Length != 0 {
		t.Fatalf("expected clamped empty read: %+v", resp)
	}
}

func TestHandleSessionLogRejectsPathOutsideLogDir(t *testing.T) {
	s, _ := newSessionLogTestServer(t)
	outside := filepath.Join(t.TempDir(), "secret.log")
	if err := os.WriteFile(outside, []byte("secret"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[1] = &session{ID: 1, LogPath: outside}
	w := sessionLogGet(t, s, "/api/session-log?token=tok&session_id=1")
	if w.Code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", w.Code, w.Body.String())
	}
}

func TestHandleSessionLogMasksSecrets(t *testing.T) {
	s, sessionsDir := newSessionLogTestServer(t)
	logPath := filepath.Join(sessionsDir, "claude_x_s2.log")
	secret := "sk-ant-abcdefghijklmnopqrstuvwxyz123456"
	if err := os.WriteFile(logPath, []byte("key="+secret+" end"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[2] = &session{ID: 2, LogPath: logPath}
	w := sessionLogGet(t, s, "/api/session-log?token=tok&session_id=2")
	resp := decodeSessionLogResp(t, w)
	data, _ := base64.StdEncoding.DecodeString(resp.DataB64)
	if string(data) == "" || strings.Contains(string(data), secret) {
		t.Fatalf("secret should be masked: %q", data)
	}
}

package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

func TestAuthRevokeAll_RotatesToken(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Token = "oldtok"

	req := httptest.NewRequest(http.MethodPost, "/api/auth/revoke-all?token=oldtok", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleAuthRevokeAll(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}
	var resp map[string]any
	if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
		t.Fatalf("decode: %v", err)
	}
	newToken, _ := resp["token"].(string)
	if newToken == "" || newToken == "oldtok" {
		t.Fatalf("token = %q, want a fresh non-empty token", newToken)
	}
	if s.cfg.Token != newToken {
		t.Errorf("cfg.Token = %q, want %q", s.cfg.Token, newToken)
	}
	if s.cfg.AuthCookieSecret == "" {
		t.Errorf("AuthCookieSecret not set after revoke")
	}

	// 旧 token は失効済み: /api/mobile-connect が 401。
	reqOld := httptest.NewRequest(http.MethodGet, "/api/mobile-connect?token=oldtok", nil)
	reqOld.Host = "127.0.0.1:47777"
	wOld := httptest.NewRecorder()
	s.handleMobileConnect(wOld, reqOld)
	if wOld.Code != http.StatusUnauthorized {
		t.Errorf("old token code = %d, want 401", wOld.Code)
	}

	// 新 token は通る。
	reqNew := httptest.NewRequest(http.MethodGet, "/api/mobile-connect?token="+newToken, nil)
	reqNew.Host = "127.0.0.1:47777"
	wNew := httptest.NewRecorder()
	s.handleMobileConnect(wNew, reqNew)
	if wNew.Code != http.StatusOK {
		t.Errorf("new token code = %d, want 200", wNew.Code)
	}
}

func TestAuthRevokeAll_RejectsGet(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	req := httptest.NewRequest(http.MethodGet, "/api/auth/revoke-all?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleAuthRevokeAll(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Fatalf("code = %d, want 405", w.Code)
	}
}

func TestAuthRevokeAll_RejectsMissingToken(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	req := httptest.NewRequest(http.MethodPost, "/api/auth/revoke-all", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleAuthRevokeAll(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("code = %d, want 401", w.Code)
	}
}

// TestAuthRevokeAll_RejectsLogicallyRemoteWithoutPIN は PIN 未設定時の
// bootstrap で論理リモート token 保持者が revoke-all を奪えないことを確認する
// （sec-auth-03）。
func TestAuthRevokeAll_RejectsLogicallyRemoteWithoutPIN(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Token = "oldtok"
	s.cfg.Hub.AllowedHosts = []string{"hub.example.ts.net"}

	req := httptest.NewRequest(http.MethodPost, "/api/auth/revoke-all?token=oldtok", nil)
	req.Host = "hub.example.ts.net" // 論理リモート
	req.RemoteAddr = "127.0.0.1:54321"
	req.Header.Set("Origin", "https://hub.example.ts.net")
	w := httptest.NewRecorder()
	s.handleAuthRevokeAll(w, req)
	if w.Code != http.StatusForbidden {
		t.Fatalf("code = %d, want 403", w.Code)
	}
	if s.cfg.Token != "oldtok" {
		t.Errorf("token rotated by logically-remote caller: %q (want unchanged)", s.cfg.Token)
	}
}

func TestAuthRevokeAllInvalidatesUIWithoutClosingWrapper(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Token = "oldtok"
	uiWS := &websocket.Conn{}
	ui := newUIConn(uiWS)
	ui.sendFunc = func(any) error { return nil }
	ui.closeFunc = func() {}
	wrapper := newWrapperConn(nil)
	s.sessionsMu.Lock()
	s.uis[uiWS] = ui
	s.wrappers[7] = wrapper
	s.sessionsMu.Unlock()

	req := httptest.NewRequest(http.MethodPost, "/api/auth/revoke-all?token=oldtok", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleAuthRevokeAll(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200: %s", w.Code, w.Body.String())
	}

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if len(s.uis) != 0 {
		t.Fatalf("UI connections = %d, want 0 after revoke-all", len(s.uis))
	}
	if s.wrappers[7] != wrapper {
		t.Fatal("revoke-all closed or replaced the provider wrapper")
	}
	if err := ui.sendWithDeadline(proto.Message{Type: "after-revoke"}, time.Now().Add(time.Second)); err == nil {
		t.Fatal("invalidated UI accepted an outbound frame")
	}
}

func TestAddUIWithHistoryRejectsStaleAuthEpoch(t *testing.T) {
	s := newTestServer()
	s.authEpoch = 2
	uiWS := &websocket.Conn{}

	ui, history := s.addUIWithHistoryAtEpoch(uiWS, 0, 1)
	if ui != nil || history != nil {
		t.Fatal("stale authentication epoch was registered")
	}
	if len(s.uis) != 0 {
		t.Fatal("stale UI registration changed the live UI set")
	}
}

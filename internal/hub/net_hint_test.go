package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// callNetHint は /api/net-hint へ POST し、ステータスコードを返す。
// launcher 経由の正規呼び出しを模すため RemoteAddr を loopback に設定する。
func callNetHint(t *testing.T, s *Server, payload map[string]any) int {
	t.Helper()
	body, _ := json.Marshal(payload)
	req := httptest.NewRequest(http.MethodPost, "/api/net-hint?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleNetHint(w, req)
	return w.Code
}

// callInfo は /api/info を呼び、レスポンス JSON を返す。
func callInfo(t *testing.T, s *Server) map[string]any {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/info?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleInfo(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("/api/info code = %d, want 200", w.Code)
	}
	var resp map[string]any
	if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
		t.Fatalf("decode /api/info: %v", err)
	}
	return resp
}

// TestNetHint_OverridesInfo は launcher が登録した接続元情報が
// /api/info の ssh / host_ip に反映されることを確認する。
func TestNetHint_OverridesInfo(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	if code := callNetHint(t, s, map[string]any{"ssh": true, "host_label": "203.0.113.10", "env_kind": "remote-tunnel"}); code != http.StatusOK {
		t.Fatalf("/api/net-hint code = %d, want 200", code)
	}

	resp := callInfo(t, s)
	if resp["ssh"] != true {
		t.Errorf("ssh = %v, want true", resp["ssh"])
	}
	if resp["host_ip"] != "203.0.113.10" {
		t.Errorf("host_ip = %v, want %q", resp["host_ip"], "203.0.113.10")
	}
	if resp["env_kind"] != "remote-tunnel" {
		t.Errorf("env_kind = %v, want remote-tunnel", resp["env_kind"])
	}
	if resp["env_host_label"] != "203.0.113.10" {
		t.Errorf("env_host_label = %v, want %q", resp["env_host_label"], "203.0.113.10")
	}
}

// TestNetHint_EmptyHostKeepsLocalDetection は host_label 空のヒントでは
// host_ip が Hub 自身の検出値（localIP 等）のまま維持されることを確認する。
func TestNetHint_EmptyHostKeepsLocalDetection(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	if code := callNetHint(t, s, map[string]any{"ssh": true, "host_label": ""}); code != http.StatusOK {
		t.Fatalf("/api/net-hint code = %d, want 200", code)
	}

	s.netHintMu.Lock()
	gotSSH, gotHost := s.netHintSSH, s.netHintHost
	s.netHintMu.Unlock()
	if !gotSSH || gotHost != "" {
		t.Errorf("netHint = (%v, %q), want (true, \"\")", gotSSH, gotHost)
	}
}

// TestNetHint_RejectGet は GET メソッドを拒否することを確認する。
func TestNetHint_RejectGet(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	req := httptest.NewRequest(http.MethodGet, "/api/net-hint?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleNetHint(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Fatalf("code = %d, want 405", w.Code)
	}
}

// TestNetHint_RejectLogicallyRemote は launcher 以外（tailscale serve 経由など
// 論理的にリモートな呼び出し元）からの POST を 403 で拒否することを確認する
// （sec-auth-04: 環境バッジ spoofing 防止）。
func TestNetHint_RejectLogicallyRemote(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.cfg.Hub.AllowedHosts = []string{"hub.example.ts.net"}

	body, _ := json.Marshal(map[string]any{"ssh": true, "host_label": "spoof"})
	req := httptest.NewRequest(http.MethodPost, "/api/net-hint?token=tok", bytes.NewReader(body))
	req.Host = "hub.example.ts.net" // 論理リモート（loopback peer だが Host は tailnet）
	req.RemoteAddr = "127.0.0.1:54321"
	req.Header.Set("Origin", "https://hub.example.ts.net")
	w := httptest.NewRecorder()
	s.handleNetHint(w, req)
	if w.Code != http.StatusForbidden {
		t.Fatalf("code = %d, want 403", w.Code)
	}
	// netHint 状態が書き換わっていないことを確認。
	s.netHintMu.Lock()
	gotHost := s.netHintHost
	s.netHintMu.Unlock()
	if gotHost == "spoof" {
		t.Errorf("netHintHost was overwritten by logically-remote caller: %q", gotHost)
	}
}

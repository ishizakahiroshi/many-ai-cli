package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// callMobileConnect は /api/mobile-connect を token 付き GET で呼び、レスポンスを返す。
func callMobileConnect(t *testing.T, s *Server) (*httptest.ResponseRecorder, map[string]any) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/mobile-connect?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	var resp map[string]any
	if w.Code == http.StatusOK {
		if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
			t.Fatalf("decode /api/mobile-connect: %v", err)
		}
	}
	return w, resp
}

func TestMobileConnect_ReturnsConnectionInfo(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	w, resp := callMobileConnect(t, s)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}

	// S2: token を含むためキャッシュ禁止。
	if cc := w.Header().Get("Cache-Control"); cc != "no-store" {
		t.Errorf("Cache-Control = %q, want no-store", cc)
	}

	// hub_url は 127.0.0.1（bind 不変）を指し token を含む。
	hubURL, _ := resp["hub_url"].(string)
	if !strings.Contains(hubURL, "127.0.0.1:47777") {
		t.Errorf("hub_url = %q, want to contain 127.0.0.1:47777", hubURL)
	}
	if !strings.Contains(hubURL, "token=tok") {
		t.Errorf("hub_url = %q, want to contain token=tok", hubURL)
	}

	// ssh_command は 127.0.0.1 へのローカルフォワード。
	sshCmd, _ := resp["ssh_command"].(string)
	if !strings.HasPrefix(sshCmd, "ssh -L 47777:127.0.0.1:47777 ") {
		t.Errorf("ssh_command = %q, want prefix 'ssh -L 47777:127.0.0.1:47777 '", sshCmd)
	}

	if got, ok := resp["hub_port"].(float64); !ok || int(got) != 47777 {
		t.Errorf("hub_port = %v, want 47777", resp["hub_port"])
	}
	if got, ok := resp["ssh_port"].(float64); !ok || int(got) != 22 {
		t.Errorf("ssh_port = %v, want 22", resp["ssh_port"])
	}
	if _, ok := resp["lan_ip"].(string); !ok {
		t.Errorf("lan_ip missing or not a string: %v", resp["lan_ip"])
	}
}

func TestMobileConnect_RejectsMissingToken(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	req := httptest.NewRequest(http.MethodGet, "/api/mobile-connect", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("code = %d, want 401", w.Code)
	}
}

func TestMobileConnect_RejectsPost(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	req := httptest.NewRequest(http.MethodPost, "/api/mobile-connect?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Fatalf("code = %d, want 405", w.Code)
	}
}

// TestDocumentCSP_NoAllowedHosts は allowed_hosts 未設定なら loopback のみの
// connect-src で、VPN-IP のソースが含まれないことを確認する（C5）。
func TestDocumentCSP_NoAllowedHosts(t *testing.T) {
	s := newTestServer()
	csp := s.documentCSP()
	if !strings.Contains(csp, "connect-src 'self' ws://127.0.0.1:* ws://localhost:*") {
		t.Errorf("csp connect-src base missing: %q", csp)
	}
	if strings.Contains(csp, "100.101.102.103") {
		t.Errorf("csp unexpectedly contains a VPN host: %q", csp)
	}
}

// TestDocumentCSP_AllowedHostExpands は allowed_hosts の host が
// ws:// と wss:// の両方で connect-src に展開されることを確認する（C5 / G2）。
func TestDocumentCSP_AllowedHostExpands(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowedHosts = []string{"100.101.102.103", "my-pc.tailnet.ts.net"}
	csp := s.documentCSP()
	for _, want := range []string{
		"ws://100.101.102.103:*",
		"wss://100.101.102.103:*",
		"ws://my-pc.tailnet.ts.net:*",
		"wss://my-pc.tailnet.ts.net:*",
	} {
		if !strings.Contains(csp, want) {
			t.Errorf("csp missing %q in %q", want, csp)
		}
	}
}

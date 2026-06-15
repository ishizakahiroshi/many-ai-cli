package hub

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

// TestGuardGetEnforcesHostWhenTokenBypassed は、allow_loopback_without_token=true で
// トークンが省略可能な場合でも、DNS リバインディング由来の許可外ホスト名
// （Host=evil.example）への GET が拒否されることを検証する。
// loopback バイパスは「論理的にローカル」（既定ホスト）な要求のみに適用されるよう
// 厳格化したため、Host が非既定の要求はバイパス対象外になり token 層で 401 になる
// （host 層の 403 ではなく token 層で先に弾かれるが、いずれも拒否でDNSリバインド防御は成立）。
func TestGuardGetEnforcesHostWhenTokenBypassed(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowLoopbackWithoutToken = true

	req := httptest.NewRequest(http.MethodGet, "http://evil.example:47777/api/files-content", nil)
	// DNS リバインドで 127.0.0.1 に解決された攻撃者ページからのアクセスを模す。
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()

	if s.guard(w, req, http.MethodGet) {
		t.Fatal("expected DNS-rebinding GET (Host=evil.example) to be rejected even with token bypass")
	}
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusUnauthorized)
	}
}

// TestGuardGetAllowsLoopbackHostWhenTokenBypassed は、トークン省略可の構成でも
// 正規のループバック Host（Host=127.0.0.1:<port>）への GET は不変で通過することを検証する。
func TestGuardGetAllowsLoopbackHostWhenTokenBypassed(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowLoopbackWithoutToken = true

	req := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:47777/api/files-content", nil)
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()

	if !s.guard(w, req, http.MethodGet) {
		t.Fatalf("expected loopback-host GET to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

// TestGuardGetWithValidTokenBlocksDisallowedHost は、有効トークンを提示した GET でも
// Host 許可リスト外（Host=evil.example）は 403 になること（broad host-check）を検証する。
// 全 GET への Host 検証パリティ化（C4）により narrow 前提から broad 前提へ更新。
func TestGuardGetWithValidTokenBlocksDisallowedHost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())

	req := httptest.NewRequest(http.MethodGet, "http://evil.example:47777/api/files-content?token=tok", nil)
	w := httptest.NewRecorder()

	if s.guard(w, req, http.MethodGet) {
		t.Fatal("expected token-authenticated GET with Host=evil.example to be rejected (broad host-check)")
	}
	if w.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusForbidden)
	}
}

// TestGuardGetAllowsConfiguredAllowedHost は、allowed_hosts に登録した正規ホスト名への
// GET が通過することを検証する（非GET と同条件になるだけで現行機能を壊さない）。
func TestGuardGetAllowsConfiguredAllowedHost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowedHosts = []string{"10.8.0.1"}

	req := httptest.NewRequest(http.MethodGet, "http://10.8.0.1:47777/api/files-content?token=tok", nil)
	w := httptest.NewRecorder()

	if !s.guard(w, req, http.MethodGet) {
		t.Fatalf("expected GET to configured allowed host to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

// TestMethodAllowsStateChange は safe メソッド（GET/HEAD/OPTIONS）にのみ CSRF 追加チェックを
// 課さないこと（＝それ以外は課す）を直接検証する。
func TestMethodAllowsStateChange(t *testing.T) {
	cases := map[string]bool{
		http.MethodGet:     false,
		http.MethodHead:    false,
		http.MethodOptions: false,
		http.MethodPost:    true,
		http.MethodPut:     true,
		http.MethodPatch:   true,
		http.MethodDelete:  true,
	}
	for method, want := range cases {
		if got := methodAllowsStateChange(method); got != want {
			t.Fatalf("methodAllowsStateChange(%q) = %v, want %v", method, got, want)
		}
	}
}

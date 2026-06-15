package hub

import (
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"log/slog"
	"many-ai-cli/internal/config"
)

// auditFixServer builds a minimal Server with token="tok" and the given
// loopback-bypass setting, mirroring newSecTestServer but configurable so the
// bypass-leak path can be exercised.
func auditFixServer(t *testing.T, allowBypass bool) *Server {
	t.Helper()
	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = 47777
	cfg.Hub.AllowLoopbackWithoutToken = allowBypass
	return &Server{
		cfg:      cfg,
		logger:   slog.Default(),
		hubCWD:   t.TempDir(),
		sessions: map[int]*session{},
	}
}

// --- #12: 認証 Cookie の失効期限 + バイパス時のトークン非配布 ---

// TestHandleIndexTokenCookieHasMaxAge は有効トークン提示時の Set-Cookie が
// 永続セッション Cookie ではなく MaxAge/Expires を持つことを確認する。
func TestHandleIndexTokenCookieHasMaxAge(t *testing.T) {
	s := auditFixServer(t, false)
	req := httptest.NewRequest(http.MethodGet, "/?token=tok", nil)
	req.Host = "127.0.0.1:47777" // handleIndex は guardBase で Host 許可リスト検証も通す
	w := httptest.NewRecorder()
	s.handleIndex(w, req)
	var found *http.Cookie
	for _, c := range w.Result().Cookies() {
		if c.Name == tokenCookieName {
			found = c
			break
		}
	}
	if found == nil {
		t.Fatal("expected Set-Cookie for valid token request")
	}
	if found.MaxAge <= 0 {
		t.Fatalf("cookie MaxAge = %d, want > 0 (non-session cookie)", found.MaxAge)
	}
	if want := int(tokenCookieMaxAge / time.Second); found.MaxAge != want {
		t.Fatalf("cookie MaxAge = %d, want %d", found.MaxAge, want)
	}
}

// TestHandleIndexNoCookieOnLoopbackBypass は allow_loopback_without_token 有効時に
// トークン未提示の loopback 要求（バイパス成功）へは全権トークンを Set-Cookie で
// 配布しないことを確認する（生 HTTP 応答からの実トークン採取を防ぐ）。
func TestHandleIndexNoCookieOnLoopbackBypass(t *testing.T) {
	s := auditFixServer(t, true)
	// トークン未提示の loopback 要求（実ローカルアクセス＝既定ホスト）。バイパスで通過する。
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "127.0.0.1:54321"
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleIndex(w, req)
	if w.Code == http.StatusUnauthorized {
		t.Fatalf("expected bypass to pass requireToken, got 401")
	}
	for _, c := range w.Result().Cookies() {
		if c.Name == tokenCookieName {
			t.Fatal("token cookie must not be issued to a bypass-only (no-token) request")
		}
	}
}

// TestHandleIndexCookieIssuedWhenValidTokenWithBypassEnabled はバイパス有効でも
// 実トークンを提示した要求には従来どおり Cookie を発行することを確認する。
func TestHandleIndexCookieIssuedWhenValidTokenWithBypassEnabled(t *testing.T) {
	s := auditFixServer(t, true)
	req := httptest.NewRequest(http.MethodGet, "/?token=tok", nil)
	req.RemoteAddr = "127.0.0.1:54321"
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleIndex(w, req)
	var found *http.Cookie
	for _, c := range w.Result().Cookies() {
		if c.Name == tokenCookieName {
			found = c
			break
		}
	}
	if found == nil {
		t.Fatal("expected Set-Cookie when a valid token is presented")
	}
	if found.Value != "tok" {
		t.Fatalf("cookie value = %q, want %q", found.Value, "tok")
	}
}

// --- #4: uiConn 書き込みデッドライン ---

// TestUIConnSendWithDeadlineConstant は broadcastWriteTimeout が正の値であることと、
// uiConn が sendWithDeadline を提供することをコンパイル時に固定する（回帰防止）。
// 実際の WS 書き込みブロックは net 層の挙動のためここでは検証しない。
func TestBroadcastWriteTimeoutPositive(t *testing.T) {
	if broadcastWriteTimeout <= 0 {
		t.Fatalf("broadcastWriteTimeout = %v, want > 0", broadcastWriteTimeout)
	}
	// uiConn.sendWithDeadline がメソッドとして存在することを型レベルで保証する。
	var _ func(*uiConn, any, time.Time) error = (*uiConn).sendWithDeadline
}

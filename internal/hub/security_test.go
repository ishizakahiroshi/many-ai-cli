package hub

import (
	"bytes"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"

	"any-ai-cli/internal/config"
	"golang.org/x/net/websocket"
	"log/slog"
)

// newSecTestServer は token="tok"、hubCWD=<tmp> の最小 Server を生成する。
func newSecTestServer(t *testing.T, cwd string) *Server {
	t.Helper()
	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = 47777
	s := &Server{
		cfg:      cfg,
		logger:   slog.Default(),
		hubCWD:   cwd,
		sessions: map[int]*session{},
	}
	return s
}

func setSecTestHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return home
}

// --- C7: requireToken（定数時間比較） ---

func TestRequireToken_Valid(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/?token=tok", nil)
	w := httptest.NewRecorder()
	if !s.requireToken(w, req) {
		t.Fatal("expected valid token to pass")
	}
}

func TestRequireToken_AuthorizationBearer(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.Header.Set("Authorization", "Bearer tok")
	w := httptest.NewRecorder()
	if !s.requireToken(w, req) {
		t.Fatal("expected bearer token to pass")
	}
}

func TestRequireToken_Cookie(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(&http.Cookie{Name: tokenCookieName, Value: "tok"})
	w := httptest.NewRecorder()
	if !s.requireToken(w, req) {
		t.Fatal("expected cookie token to pass")
	}
}

func TestRequireToken_CookieInvalid(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.AddCookie(&http.Cookie{Name: tokenCookieName, Value: "bad"})
	w := httptest.NewRecorder()
	if s.requireToken(w, req) {
		t.Fatal("expected invalid cookie token to fail")
	}
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

func TestHandleIndexSetsTokenCookie(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/?token=tok", nil)
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
		t.Fatal("expected Set-Cookie for token cookie")
	}
	if found.Value != "tok" {
		t.Fatalf("cookie value = %q, want %q", found.Value, "tok")
	}
	if !found.HttpOnly {
		t.Fatal("expected HttpOnly cookie")
	}
	if found.SameSite != http.SameSiteStrictMode {
		t.Fatalf("SameSite = %v, want Strict", found.SameSite)
	}
}

func TestHandleIndexNoCookieWhenUnauthorized(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/?token=bad", nil)
	w := httptest.NewRecorder()
	s.handleIndex(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
	for _, c := range w.Result().Cookies() {
		if c.Name == tokenCookieName {
			t.Fatal("unexpected Set-Cookie on unauthorized request")
		}
	}
}

func TestRequireToken_Invalid(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/?token=bad", nil)
	w := httptest.NewRecorder()
	if s.requireToken(w, req) {
		t.Fatal("expected invalid token to fail")
	}
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

func TestRequireTokenLoopbackBypassDefaultOff(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "127.0.0.1:34567"
	w := httptest.NewRecorder()
	if s.requireToken(w, req) {
		t.Fatal("expected tokenless loopback request to fail while bypass is disabled")
	}
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusUnauthorized)
	}
}

func TestRequireTokenLoopbackBypassEnabled(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "127.0.0.1:34567"
	w := httptest.NewRecorder()
	if !s.requireToken(w, req) {
		t.Fatalf("expected tokenless loopback request to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

func TestRequireTokenTrustedNetworkBypassEnabled(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Hub.TrustedNetworks = []string{"172.19.0.1/32"}
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "172.19.0.1:34567"
	w := httptest.NewRecorder()
	if !s.requireToken(w, req) {
		t.Fatalf("expected trusted network request to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

func TestRequireTokenBypassRejectsUntrustedRemote(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Hub.TrustedNetworks = []string{"172.19.0.1/32"}
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.RemoteAddr = "203.0.113.10:34567"
	w := httptest.NewRecorder()
	if s.requireToken(w, req) {
		t.Fatal("expected tokenless untrusted request to fail")
	}
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusUnauthorized)
	}
}

func TestAllowedHubHost(t *testing.T) {
	cases := []struct {
		host string
		port int
		want bool
	}{
		{"127.0.0.1:47777", 47777, true},
		{"localhost:47777", 47777, true},
		{"[::1]:47777", 47777, true},
		{"127.0.0.1:47778", 47777, false},
		{"evil.example:47777", 47777, false},
	}
	for _, c := range cases {
		if got := isAllowedHubHost(c.host, c.port); got != c.want {
			t.Fatalf("isAllowedHubHost(%q, %d) = %v, want %v", c.host, c.port, got, c.want)
		}
	}
}

func TestAllowedHubHostConfig(t *testing.T) {
	if got := isAllowedHubHost("10.8.0.1:47777", 47777); got {
		t.Fatal("isAllowedHubHost without configured allowed host = true, want false")
	}
	if got := isAllowedHubHost("10.8.0.1:47777", 47777, "10.8.0.1"); !got {
		t.Fatal("isAllowedHubHost with configured allowed host = false, want true")
	}
}

func TestAllowedHubOrigin(t *testing.T) {
	cases := []struct {
		origin string
		port   int
		want   bool
	}{
		{"http://127.0.0.1:47777", 47777, true},
		{"http://localhost:47777", 47777, true},
		{"http://[::1]:47777", 47777, true},
		{"https://127.0.0.1:47777", 47777, false},
		{"http://127.0.0.1:47778", 47777, false},
		{"http://evil.example:47777", 47777, false},
	}
	for _, c := range cases {
		if got := isAllowedHubOrigin(c.origin, c.port); got != c.want {
			t.Fatalf("isAllowedHubOrigin(%q, %d) = %v, want %v", c.origin, c.port, got, c.want)
		}
	}
}

func TestAllowedHubOriginConfig(t *testing.T) {
	if got := isAllowedHubOrigin("http://10.8.0.1:47777", 47777); got {
		t.Fatal("isAllowedHubOrigin without configured allowed host = true, want false")
	}
	if got := isAllowedHubOrigin("http://10.8.0.1:47777", 47777, "10.8.0.1"); !got {
		t.Fatal("isAllowedHubOrigin with configured allowed host = false, want true")
	}
}

func TestGuardAllowsSameOriginPost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777/api/test?token=tok", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	w := httptest.NewRecorder()
	if !s.guard(w, req, http.MethodPost) {
		t.Fatalf("expected same-origin POST to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

func TestGuardRejectsUnexpectedOriginPost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777/api/test?token=tok", nil)
	req.Header.Set("Origin", "http://evil.example:47777")
	w := httptest.NewRecorder()
	if s.guard(w, req, http.MethodPost) {
		t.Fatal("expected cross-origin POST to fail")
	}
	if w.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusForbidden)
	}
}

func TestGuardRejectsCrossSiteFetchPost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777/api/test?token=tok", nil)
	req.Header.Set("Sec-Fetch-Site", "cross-site")
	w := httptest.NewRecorder()
	if s.guard(w, req, http.MethodPost) {
		t.Fatal("expected cross-site POST to fail")
	}
	if w.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusForbidden)
	}
}

func TestGuardAllowsPostWithoutBrowserOriginHeaders(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777/api/test?token=tok", nil)
	w := httptest.NewRecorder()
	if !s.guard(w, req, http.MethodPost) {
		t.Fatalf("expected CLI-style POST to pass, status=%d body=%s", w.Code, w.Body.String())
	}
}

func TestWSHandshakeRejectsUnexpectedHost(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/ws", nil)
	req.Host = "evil.example:47777"
	if err := s.wsHandshake(&websocket.Config{}, req); err == nil {
		t.Fatal("expected unexpected Host to be rejected")
	}
}

func TestWSHandshakeAllowsLoopbackHostAndOrigin(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/ws", nil)
	req.Host = "127.0.0.1:47777"
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	if err := s.wsHandshake(&websocket.Config{}, req); err != nil {
		t.Fatalf("expected loopback Host/Origin to pass: %v", err)
	}
}

func TestWSHandshakeAllowsConfiguredHostAndOrigin(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Hub.AllowedHosts = []string{"10.8.0.1"}
	req := httptest.NewRequest(http.MethodGet, "/ws", nil)
	req.Host = "10.8.0.1:47777"
	req.Header.Set("Origin", "http://10.8.0.1:47777")
	if err := s.wsHandshake(&websocket.Config{}, req); err != nil {
		t.Fatalf("expected configured Host/Origin to pass: %v", err)
	}
}

func TestWSHandshakeRejectsUnexpectedOrigin(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/ws", nil)
	req.Host = "127.0.0.1:47777"
	req.Header.Set("Origin", "http://evil.example:47777")
	if err := s.wsHandshake(&websocket.Config{}, req); err == nil {
		t.Fatal("expected unexpected Origin to be rejected")
	}
}

func TestWSHandshakeAllowsEmptyOriginForCLI(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/ws", nil)
	req.Host = "127.0.0.1:47777"
	if err := s.wsHandshake(&websocket.Config{}, req); err != nil {
		t.Fatalf("expected empty Origin to pass for CLI clients: %v", err)
	}
}

func TestRegisteredAPIRoutesRequireToken(t *testing.T) {
	cfg := &config.Config{Token: "tok"}
	cfg.Hub.Port = 47777
	cfg.Hub.LogDir = t.TempDir()
	s, err := NewServer(cfg, slog.Default(), true, "test")
	if err != nil {
		t.Fatal(err)
	}
	if s.sessionStore != nil {
		t.Cleanup(func() { _ = s.sessionStore.Close() })
	}

	for _, path := range registeredAPIRoutes(t) {
		req := httptest.NewRequest(http.MethodGet, "http://127.0.0.1:47777"+path, nil)
		req.RemoteAddr = "203.0.113.10:34567"
		w := httptest.NewRecorder()
		s.httpSrv.Handler.ServeHTTP(w, req)
		if w.Code != http.StatusUnauthorized {
			t.Fatalf("%s without token returned status %d, want %d; body=%s", path, w.Code, http.StatusUnauthorized, w.Body.String())
		}
	}
}

func registeredAPIRoutes(t *testing.T) []string {
	t.Helper()
	seen := map[string]struct{}{}
	for _, name := range []string{"server.go", "workbench_handlers.go"} {
		path := filepath.Join("..", "..", "internal", "hub", name)
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			t.Fatalf("parse %s: %v", path, err)
		}
		ast.Inspect(file, func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok || len(call.Args) == 0 {
				return true
			}
			sel, ok := call.Fun.(*ast.SelectorExpr)
			if !ok || (sel.Sel.Name != "HandleFunc" && sel.Sel.Name != "Handle") {
				return true
			}
			lit, ok := call.Args[0].(*ast.BasicLit)
			if !ok || lit.Kind != token.STRING {
				return true
			}
			value, err := strconv.Unquote(lit.Value)
			if err == nil && strings.HasPrefix(value, "/api/") {
				seen[value] = struct{}{}
			}
			return true
		})
	}
	if len(seen) == 0 {
		t.Fatal("no /api routes found")
	}
	routes := make([]string, 0, len(seen))
	for route := range seen {
		routes = append(routes, route)
	}
	return routes
}

func TestRequireToken_Empty(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	w := httptest.NewRecorder()
	if s.requireToken(w, req) {
		t.Fatal("expected empty token to fail")
	}
}

func TestDecodeJSONRejectsOversizedBody(t *testing.T) {
	req := httptest.NewRequest(http.MethodPost, "/", strings.NewReader(strings.Repeat(" ", jsonBodyMaxBytes+1)))
	w := httptest.NewRecorder()
	var dst map[string]any
	if decodeJSON(w, req, &dst) {
		t.Fatal("expected oversized JSON body to fail")
	}
	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusBadRequest)
	}
}

func TestSecurityHeadersMiddleware(t *testing.T) {
	handler := withSecurityHeaders(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNoContent)
	}))
	w := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	handler.ServeHTTP(w, req)

	if got := w.Header().Get("X-Frame-Options"); got != "DENY" {
		t.Fatalf("X-Frame-Options = %q, want DENY", got)
	}
	if got := w.Header().Get("Referrer-Policy"); got != "no-referrer" {
		t.Fatalf("Referrer-Policy = %q, want no-referrer", got)
	}
	if got := w.Header().Get("X-Content-Type-Options"); got != "nosniff" {
		t.Fatalf("X-Content-Type-Options = %q, want nosniff", got)
	}
	if got := w.Header().Get("Cross-Origin-Opener-Policy"); got != "same-origin" {
		t.Fatalf("Cross-Origin-Opener-Policy = %q, want same-origin", got)
	}
	if got := w.Header().Get("Permissions-Policy"); !strings.Contains(got, "microphone=(self)") {
		t.Fatalf("Permissions-Policy = %q, want microphone=(self)", got)
	}
	if got := w.Header().Get("Content-Security-Policy"); got == "" {
		t.Fatal("Content-Security-Policy header should be set")
	}
}

func TestLimitWSReceive(t *testing.T) {
	conn := &websocket.Conn{}
	limitWSReceive(conn)
	if conn.MaxPayloadBytes != wsMaxPayloadBytes {
		t.Fatalf("MaxPayloadBytes = %d, want %d", conn.MaxPayloadBytes, wsMaxPayloadBytes)
	}
}

func TestApprovalPatternAssetRequiresToken(t *testing.T) {
	setSecTestHome(t)
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, config.DirMode); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "codex.json"), []byte(`["approve?"]`), 0o600); err != nil {
		t.Fatal(err)
	}
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/approval-patterns/codex.json", nil)
	w := httptest.NewRecorder()
	s.handleApprovalPatternAsset(w, req)

	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusUnauthorized)
	}
}

func TestApprovalPatternAssetServesKnownFileWithToken(t *testing.T) {
	setSecTestHome(t)
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, config.DirMode); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "codex.json"), []byte(`["approve?"]`), 0o600); err != nil {
		t.Fatal(err)
	}
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/approval-patterns/codex.json?token=tok", nil)
	w := httptest.NewRecorder()
	s.handleApprovalPatternAsset(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d: %s", w.Code, http.StatusOK, w.Body.String())
	}
	if got := w.Header().Get("X-Content-Type-Options"); got != "nosniff" {
		t.Fatalf("X-Content-Type-Options = %q, want nosniff", got)
	}
	if !strings.Contains(w.Body.String(), "approve?") {
		t.Fatalf("expected approval pattern body, got: %s", w.Body.String())
	}
}

func TestApprovalPatternAssetRejectsTraversal(t *testing.T) {
	setSecTestHome(t)
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/approval-patterns/../config.yaml?token=tok", nil)
	w := httptest.NewRecorder()
	s.handleApprovalPatternAsset(w, req)

	if w.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusNotFound)
	}
}

func TestHandleAttachRejectsOversizedFile(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.sessions[1] = &session{ID: 1, Provider: "codex"}
	var body bytes.Buffer
	mw := multipart.NewWriter(&body)
	if err := mw.WriteField("session_id", "1"); err != nil {
		t.Fatal(err)
	}
	part, err := mw.CreateFormFile("file", "large.txt")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := part.Write(bytes.Repeat([]byte("x"), attachUploadMaxBytes+1)); err != nil {
		t.Fatal(err)
	}
	if err := mw.Close(); err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequest(http.MethodPost, "/api/attach?token=tok", &body)
	req.Host = "127.0.0.1:47777"
	req.Header.Set("Content-Type", mw.FormDataContentType())
	w := httptest.NewRecorder()
	s.handleAttach(w, req)

	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d: %s", w.Code, http.StatusBadRequest, w.Body.String())
	}
}

// --- C3: validRevision ---

func TestValidRevision(t *testing.T) {
	cases := []struct {
		input string
		want  bool
	}{
		// 正常ケース
		{"abc123", true},
		{"HEAD", true},
		{"develop", true},
		{"origin/main", true},
		{"feature/foo-bar", true},
		{"v1.0.3", true},
		{"abcdef1234567890", true},
		// 異常ケース: 先頭 "-"
		{"-x", false},
		{"--output=evil", false},
		{"-", false},
		// 空文字
		{"", false},
		// 不正文字
		{"ab;cd", false},
		{"ab cd", false},
		{"ab\x00cd", false},
		{"ab`cd", false},
		{"ab$cd", false},
	}
	for _, c := range cases {
		got := validRevision(c.input)
		if got != c.want {
			t.Errorf("validRevision(%q) = %v, want %v", c.input, got, c.want)
		}
	}
}

// --- C6: isPathUnderAllowedRoots / isUnder ---

func TestIsPathUnderAllowedRoots_Basic(t *testing.T) {
	tmp := t.TempDir()
	sub := filepath.Join(tmp, "sub", "dir")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}

	ok, err := isPathUnderAllowedRoots(sub, tmp)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("expected sub to be under tmp")
	}
}

func TestIsPathUnderAllowedRoots_Outside(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()
	sub := filepath.Join(outside, "evil")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}

	ok, err := isPathUnderAllowedRoots(sub, tmp)
	if err != nil {
		t.Fatal(err)
	}
	if ok {
		t.Fatal("expected outside path to be rejected")
	}
}

func TestIsPathUnderAllowedRoots_DotDotEscape(t *testing.T) {
	tmp := t.TempDir()
	// path traversal: tmp/sub/../../etc
	traversal := filepath.Join(tmp, "sub", "..", "..", "etc")
	ok, err := isPathUnderAllowedRoots(traversal, tmp)
	if err != nil {
		t.Fatal(err)
	}
	if ok {
		t.Fatalf("expected ../ escape to be rejected: path=%s, root=%s", traversal, tmp)
	}
}

func TestIsPathUnderAllowedRoots_NonexistentChild(t *testing.T) {
	tmp := t.TempDir()
	// 存在しない子パスも配下なら許可
	child := filepath.Join(tmp, "new_file.txt")

	ok, err := isPathUnderAllowedRoots(child, tmp)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("expected nonexistent child to be under tmp")
	}
}

func TestIsPathUnderAllowedRoots_DotDotPrefixName(t *testing.T) {
	tmp := t.TempDir()
	child := filepath.Join(tmp, "..not-traversal")
	if err := os.MkdirAll(child, 0o755); err != nil {
		t.Fatal(err)
	}

	ok, err := isPathUnderAllowedRoots(child, tmp)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("expected child name beginning with '..' to be accepted under tmp")
	}
}

func TestIsUnder_CaseSensitivity(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("Windows-only test")
	}
	// Windows: 大文字小文字違いでも配下と認識されるべき
	base := `C:\Users\test`
	target := `C:\users\TEST\sub`
	if !isUnder(target, base) {
		t.Fatalf("expected case-insensitive match on Windows: target=%s, base=%s", target, base)
	}
}

func TestHandlePathExistsOmitsUnknownPaths(t *testing.T) {
	root := t.TempDir()
	historyDir := filepath.Join(root, "known")
	if err := os.MkdirAll(historyDir, 0o755); err != nil {
		t.Fatal(err)
	}
	outside := t.TempDir()

	s := newSecTestServer(t, root)
	s.cfg.UserPrefs.CwdHistory = []string{historyDir}
	body := []byte(`{"paths":[` + strconvQuote(historyDir) + `,` + strconvQuote(outside) + `]}`)
	req := httptest.NewRequest(http.MethodPost, "/api/path-exists?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handlePathExists(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", w.Code, w.Body.String())
	}
	var resp struct {
		Results map[string]bool `json:"results"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if !resp.Results[historyDir] {
		t.Fatalf("known history dir should be reported as existing: %+v", resp.Results)
	}
	if _, ok := resp.Results[outside]; ok {
		t.Fatalf("unknown path should be omitted from oracle response: %+v", resp.Results)
	}
}

func TestHandleOpenDirPathRejectsOutsideAllowedRoots(t *testing.T) {
	root := t.TempDir()
	outside := filepath.Join(t.TempDir(), "outside.txt")
	if err := os.WriteFile(outside, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	s := newSecTestServer(t, root)
	body := []byte(`{"kind":"path","path":` + strconvQuote(outside) + `}`)
	req := httptest.NewRequest(http.MethodPost, "/api/open-dir?token=tok", bytes.NewReader(body))
	req.RemoteAddr = "127.0.0.1:12345"
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleOpenDir(w, req)

	if w.Code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", w.Code, w.Body.String())
	}
}

func TestCheckOpenPathAllowedUsesSessionCWD(t *testing.T) {
	hubRoot := t.TempDir()
	sessionRoot := t.TempDir()
	target := filepath.Join(sessionRoot, "child.txt")

	s := newSecTestServer(t, hubRoot)
	s.sessionsMu.Lock()
	s.sessions[7] = &session{ID: 7, CWD: sessionRoot}
	s.sessionsMu.Unlock()

	reqWithoutSession := httptest.NewRequest(http.MethodPost, "/api/open-file?token=tok", nil)
	wWithoutSession := httptest.NewRecorder()
	if s.checkOpenPathAllowed(wWithoutSession, reqWithoutSession, target) {
		t.Fatal("expected path outside Hub cwd to be rejected without session scope")
	}
	if wWithoutSession.Code != http.StatusForbidden {
		t.Fatalf("status without session = %d, want %d: %s", wWithoutSession.Code, http.StatusForbidden, wWithoutSession.Body.String())
	}

	reqWithSession := httptest.NewRequest(http.MethodPost, "/api/open-file?token=tok&session=7", nil)
	wWithSession := httptest.NewRecorder()
	if !s.checkOpenPathAllowed(wWithSession, reqWithSession, target) {
		t.Fatalf("expected path under session cwd to be allowed, status=%d body=%s", wWithSession.Code, wWithSession.Body.String())
	}
}

func strconvQuote(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}

// --- C7: sanitizeGitErrMsg ---

func TestSanitizeGitErrMsg_PathInStderr(t *testing.T) {
	err := &sanitizeGitTestErr{msg: `git show abc: fatal: /home/user/.any-ai-cli/repo: not a git repo`}
	got := sanitizeGitErrMsg(err)
	if got == "" {
		t.Fatal("expected non-empty sanitized message")
	}
	// 生の絶対パスが出力に含まれていないこと
	if containsPath(got) {
		t.Fatalf("sanitized message should not contain absolute path: %q", got)
	}
}

func TestSanitizeGitErrMsg_SafeMessage(t *testing.T) {
	err := &sanitizeGitTestErr{msg: `git log HEAD: exit status 128`}
	got := sanitizeGitErrMsg(err)
	// パス・URL を含まない場合はそのまま返す
	if got != `git log HEAD: exit status 128` {
		t.Fatalf("expected safe message to pass through, got: %q", got)
	}
}

func TestSanitizeGitErrMsg_URLInStderr(t *testing.T) {
	err := &sanitizeGitTestErr{msg: `git fetch: fatal: repository 'https://token@example.com/private/repo.git' not found`}
	got := sanitizeGitErrMsg(err)
	if strings.Contains(got, "https://") || strings.Contains(got, "token@") {
		t.Fatalf("sanitized message should not contain remote URL: %q", got)
	}
}

type sanitizeGitTestErr struct{ msg string }

func (e *sanitizeGitTestErr) Error() string { return e.msg }

func containsPath(s string) bool {
	for _, ch := range s {
		if ch == '/' || ch == '\\' {
			return true
		}
	}
	return false
}

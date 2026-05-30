package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
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

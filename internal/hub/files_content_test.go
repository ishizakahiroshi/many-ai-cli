package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionstore"
)

func newTestFilesContentServer(tmpDir string) *Server {
	return &Server{
		cfg:      &config.Config{Token: "tok"},
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
}

func callFilesContent(t *testing.T, s *Server, path string) (int, string, filesContentResp) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/files-content?token=tok&path="+url.QueryEscape(path), nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesContent(w, req)

	var resp filesContentResp
	body := w.Body.String()
	_ = json.Unmarshal([]byte(body), &resp)
	return w.Code, body, resp
}

func callFilesAsset(t *testing.T, s *Server, path string) (int, string) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/files-asset?token=tok&path="+url.QueryEscape(path), nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesAsset(w, req)
	return w.Code, w.Body.String()
}

func callFilesDownload(t *testing.T, s *Server, path string) (int, string, http.Header) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/files-download?token=tok&path="+url.QueryEscape(path), nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesDownload(w, req)
	return w.Code, w.Body.String(), w.Header()
}

func callFilesDownloadWithSession(t *testing.T, s *Server, path string, sessionID int) (int, string, http.Header) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/files-download?token=tok&session="+strconv.Itoa(sessionID)+"&path="+url.QueryEscape(path), nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesDownload(w, req)
	return w.Code, w.Body.String(), w.Header()
}

func TestIsTextFile(t *testing.T) {
	cases := map[string]bool{
		"main.go":        true,
		"app.js":         true,
		"config.yaml":    true,
		"Dockerfile":     true,
		"dockerfile":     true,
		"Makefile":       true,
		"README":         true,
		".gitignore":     true,
		"image.png":      false,
		"program.exe":    false,
		"archive.tar.gz": false,
	}
	for name, want := range cases {
		if got := isTextFile(filepath.Join("root", name)); got != want {
			t.Fatalf("isTextFile(%q) = %v, want %v", name, got, want)
		}
	}
}

func TestHandleFilesContent_AllowsPreviewableText(t *testing.T) {
	tmp := t.TempDir()
	files := map[string]string{
		"main.go":    "package main\n",
		"app.js":     "console.log('ok');\n",
		"config.yml": "ok: true\n",
		"Dockerfile": "FROM scratch\n",
		"Makefile":   "test:\n\tgo test ./...\n",
	}

	s := newTestFilesContentServer(tmp)
	for name, content := range files {
		path := filepath.Join(tmp, name)
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
		code, _, resp := callFilesContent(t, s, path)
		if code != http.StatusOK {
			t.Fatalf("%s: expected 200, got %d", name, code)
		}
		if resp.Content != content {
			t.Fatalf("%s: content = %q, want %q", name, resp.Content, content)
		}
	}
}

func TestHandleFilesDownload_AllowsArbitraryExtension(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "payload.custombin")
	content := "binary-ish\x00payload"
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, header := callFilesDownload(t, s, path)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if body != content {
		t.Fatalf("body = %q, want %q", body, content)
	}
	disposition := header.Get("Content-Disposition")
	if !strings.Contains(disposition, "attachment") || !strings.Contains(disposition, `filename="payload.custombin"`) || !strings.Contains(disposition, "filename*=UTF-8''payload.custombin") {
		t.Fatalf("unexpected Content-Disposition: %q", disposition)
	}
	if got := header.Get("Content-Type"); got != "application/octet-stream" {
		t.Fatalf("Content-Type = %q, want application/octet-stream", got)
	}
	if got := header.Get("X-Content-Type-Options"); got != "nosniff" {
		t.Fatalf("X-Content-Type-Options = %q, want nosniff", got)
	}
}

func TestHandleFilesDownload_ContentDispositionNonASCII(t *testing.T) {
	tmp := t.TempDir()
	name := "日本語.txt"
	path := filepath.Join(tmp, name)
	if err := os.WriteFile(path, []byte("hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, header := callFilesDownload(t, s, path)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	disposition := header.Get("Content-Disposition")
	if !strings.Contains(disposition, `filename="___.txt"`) {
		t.Fatalf("missing ASCII fallback in Content-Disposition: %q", disposition)
	}
	if !strings.Contains(disposition, "filename*=UTF-8''"+url.PathEscape(name)) {
		t.Fatalf("missing filename* in Content-Disposition: %q", disposition)
	}
	if got := header.Get("Content-Type"); !strings.HasPrefix(got, "text/plain") {
		t.Fatalf("Content-Type = %q, want text/plain", got)
	}
}

func TestHandleFilesDownload_RejectsDirectory(t *testing.T) {
	tmp := t.TempDir()
	s := newTestFilesContentServer(tmp)

	code, body, _ := callFilesDownload(t, s, tmp)
	if code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d: %s", code, body)
	}
	if !strings.Contains(body, "path is a directory") {
		t.Fatalf("unexpected body: %q", body)
	}
}

func TestHandleFilesContent_RejectsBinaryExtension(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "image.png")
	if err := os.WriteFile(path, []byte{0x89, 'P', 'N', 'G'}, 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, _ := callFilesContent(t, s, path)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d", code)
	}
	if !strings.Contains(body, "not a previewable text file") {
		t.Fatalf("unexpected body: %q", body)
	}
}

// newMentionTestServer は「チャットに言及されたスコープ外パス」テスト用の
// Server（sessionStore + live セッション付き）と、言及登録用ヘルパを返す。
func newMentionTestServer(t *testing.T, projDir string, sessionID int) (*Server, func(text string)) {
	t.Helper()
	store, err := sessionstore.OpenForLogDir(filepath.Join(t.TempDir(), "logs"))
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	t.Cleanup(func() { store.Close() })
	if _, err := store.StartSession(sessionstore.SessionStart{
		LiveSessionID: sessionID,
		Provider:      "claude",
		CWD:           projDir,
		State:         "standby",
		StartedAt:     time.Now().Format(time.RFC3339),
	}); err != nil {
		t.Fatalf("StartSession: %v", err)
	}
	s := newTestFilesContentServer(projDir)
	s.sessionStore = store
	s.sessions[sessionID] = &session{ID: sessionID, CWD: projDir}
	mention := func(text string) {
		t.Helper()
		ev := map[string]any{"ts": time.Now().Format(time.RFC3339), "type": "pty_output", "session_id": sessionID, "text": text}
		if err := store.StoreEvent(sessionID, ev); err != nil {
			t.Fatalf("StoreEvent: %v", err)
		}
	}
	return s, mention
}

func callFilesContentWithSession(t *testing.T, s *Server, path string, sessionID int) (int, string, filesContentResp) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/files-content?token=tok&session="+strconv.Itoa(sessionID)+"&path="+url.QueryEscape(path), nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesContent(w, req)
	var resp filesContentResp
	body := w.Body.String()
	_ = json.Unmarshal([]byte(body), &resp)
	return w.Code, body, resp
}

// TestHandleFilesContent_MentionedOutsidePathReadOnly は、チャットに言及された
// スコープ外パスが読み取り専用（readOnly=true）で 200 になることを確認する。
func TestHandleFilesContent_MentionedOutsidePathReadOnly(t *testing.T) {
	projDir := t.TempDir()
	outsideDir := t.TempDir()
	outsideFile := filepath.Join(outsideDir, "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("変更ファイル: " + outsideFile + "\n")

	code, body, resp := callFilesContentWithSession(t, s, outsideFile, 7)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if !resp.ReadOnly {
		t.Fatalf("expected readOnly=true, got %+v", resp)
	}
	if resp.Content != "# outside\n" {
		t.Fatalf("content = %q", resp.Content)
	}

	// スコープ内のファイルは従来どおり readOnly=false
	insideFile := filepath.Join(projDir, "inside.md")
	if err := os.WriteFile(insideFile, []byte("inside\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	code, body, resp = callFilesContentWithSession(t, s, insideFile, 7)
	if code != http.StatusOK || resp.ReadOnly {
		t.Fatalf("inside file: code=%d readOnly=%v body=%s", code, resp.ReadOnly, body)
	}
}

// TestHandleFilesContent_UnmentionedOutsidePathForbidden は、言及のないスコープ外パスが
// 引き続き 403 のままであることを確認する。
func TestHandleFilesContent_UnmentionedOutsidePathForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "secret.md")
	if err := os.WriteFile(outsideFile, []byte("secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("関係ない出力\n")

	// session 指定あり・言及なし → 403
	code, body, _ := callFilesContentWithSession(t, s, outsideFile, 7)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", code, body)
	}
	// session 指定なし → 403
	code, body, _ = callFilesContent(t, s, outsideFile)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403 without session, got %d: %s", code, body)
	}
}

func TestHandleFilesDownload_UnmentionedOutsidePathForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "artifact.bin")
	if err := os.WriteFile(outsideFile, []byte("secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("関係ない出力\n")

	code, body, _ := callFilesDownloadWithSession(t, s, outsideFile, 7)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", code, body)
	}
}

func TestHandleFilesDownload_MentionedOutsidePathAllowed(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "artifact.bin")
	if err := os.WriteFile(outsideFile, []byte("outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("artifact: " + outsideFile + "\n")

	code, body, _ := callFilesDownloadWithSession(t, s, outsideFile, 7)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if body != "outside\n" {
		t.Fatalf("body = %q", body)
	}
}

// TestHandleFilesSave_MentionedOutsidePathStillForbidden は、言及があっても
// 書き込み系（files-save）は 403 のままであることを確認する（読み取り専用の担保）。
func TestHandleFilesSave_MentionedOutsidePathStillForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("変更ファイル: " + outsideFile + "\n")

	bodyJSON := `{"path":` + mustJSONString(t, outsideFile) + `,"content":"overwrite"}`
	req := httptest.NewRequest(http.MethodPost, "/api/files-save?token=tok&session=7", strings.NewReader(bodyJSON))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.handleFilesSave(w, req)
	if w.Code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", w.Code, w.Body.String())
	}
}

func mustJSONString(t *testing.T, v string) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestHandleFilesAsset_RejectsSVG(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "preview.svg")
	if err := os.WriteFile(path, []byte(`<svg><script>alert(1)</script></svg>`), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body := callFilesAsset(t, s, path)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d", code)
	}
	if !strings.Contains(body, "not a previewable media file") {
		t.Fatalf("unexpected body: %q", body)
	}
}

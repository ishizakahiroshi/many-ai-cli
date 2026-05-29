package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"any-ai-cli/internal/config"
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
	w := httptest.NewRecorder()
	s.handleFilesAsset(w, req)
	return w.Code, w.Body.String()
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

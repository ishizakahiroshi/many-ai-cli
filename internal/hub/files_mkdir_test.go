package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"any-ai-cli/internal/config"
)

func newTestMkdirServer(t *testing.T, tmpDir string) *Server {
	t.Helper()
	cfg := &config.Config{Token: "tok"}
	s := &Server{
		cfg:      cfg,
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
	return s
}

func callMkdir(t *testing.T, s *Server, dir, name string) (int, filesMkdirResp) {
	t.Helper()
	body, _ := json.Marshal(filesMkdirReq{Dir: dir, Name: name})
	req := httptest.NewRequest(http.MethodPost, "/api/files-mkdir?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesMkdir(w, req)
	var resp filesMkdirResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesMkdir_OK(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMkdirServer(t, tmp)

	code, resp := callMkdir(t, s, tmp, "newdir")
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	want := filepath.Join(tmp, "newdir")
	if resp.NewAbs != want {
		t.Fatalf("newAbs = %q, want %q", resp.NewAbs, want)
	}
	info, err := os.Stat(want)
	if err != nil {
		t.Fatalf("directory not created: %v", err)
	}
	if !info.IsDir() {
		t.Fatal("created entry is not a directory")
	}
}

func TestFilesMkdir_RejectOutsideAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()

	s := newTestMkdirServer(t, tmp)
	code, resp := callMkdir(t, s, outside, "newdir")
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected forbidden, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error code: %q", resp.Error)
	}
}

func TestFilesMkdir_RejectAlreadyExists(t *testing.T) {
	tmp := t.TempDir()
	existing := filepath.Join(tmp, "existing")
	if err := os.Mkdir(existing, 0o755); err != nil {
		t.Fatal(err)
	}

	s := newTestMkdirServer(t, tmp)
	code, resp := callMkdir(t, s, tmp, "existing")
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected conflict, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "already_exists" {
		t.Fatalf("unexpected error code: %q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesMkdir_RejectMissingToken(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMkdirServer(t, tmp)

	body, _ := json.Marshal(filesMkdirReq{Dir: tmp, Name: "newdir"})
	req := httptest.NewRequest(http.MethodPost, "/api/files-mkdir", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesMkdir(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

func TestFilesMkdir_RejectNonAbsoluteDir(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMkdirServer(t, tmp)

	code, resp := callMkdir(t, s, "relative/path", "newdir")
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected bad_request, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" {
		t.Fatalf("unexpected error code: %q", resp.Error)
	}
}

func TestFilesMkdir_RejectSeparatorInName(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMkdirServer(t, tmp)

	for _, bad := range []string{"a/b", `a\b`, "..", ".", "", "C:bad"} {
		code, resp := callMkdir(t, s, tmp, bad)
		if code != http.StatusBadRequest || resp.OK {
			t.Fatalf("expected bad_request for %q, got code=%d resp=%+v", bad, code, resp)
		}
	}
}

func TestFilesMkdir_RejectParentNotFound(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMkdirServer(t, tmp)

	nonexistent := filepath.Join(tmp, "does-not-exist")
	code, resp := callMkdir(t, s, nonexistent, "newdir")
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected bad_request, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" {
		t.Fatalf("unexpected error code: %q detail=%q", resp.Error, resp.Detail)
	}
}

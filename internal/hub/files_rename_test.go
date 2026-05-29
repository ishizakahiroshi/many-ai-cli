package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"any-ai-cli/internal/config"
)

func newTestRenameServer(t *testing.T, tmpDir string) *Server {
	t.Helper()
	cfg := &config.Config{Token: "tok"}
	s := &Server{
		cfg:      cfg,
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
	return s
}

func callRename(t *testing.T, s *Server, src, newName string) (int, filesRenameResp) {
	t.Helper()
	body, _ := json.Marshal(filesRenameReq{Src: src, NewName: newName})
	req := httptest.NewRequest(http.MethodPost, "/api/files-rename?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesRename(w, req)
	var resp filesRenameResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesRename_RejectFile(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "old.md")
	if err := os.WriteFile(src, []byte("hi"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "new.md")
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" || !strings.Contains(resp.Detail, "directory") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesRename_OK_Directory(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "olddir")
	if err := os.MkdirAll(src, 0o755); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "newdir")
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	want := filepath.Join(tmp, "newdir")
	if resp.NewAbs != want {
		t.Fatalf("newAbs = %q, want %q", resp.NewAbs, want)
	}
}

func TestFilesRename_RejectSameName(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "foo")
	_ = os.MkdirAll(src, 0o755)

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "foo")
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" || !strings.Contains(resp.Detail, "identical") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesRename_RejectOverwrite(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "old")
	dst := filepath.Join(tmp, "new")
	_ = os.MkdirAll(src, 0o755)
	_ = os.MkdirAll(dst, 0o755)

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "new")
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" || !strings.Contains(resp.Detail, "already exists") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesRename_RejectSeparatorInName(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "foo.md")
	_ = os.WriteFile(src, []byte("x"), 0o644)

	for _, bad := range []string{"a/b.md", `a\b.md`, "..", ".", "", "C:bad"} {
		s := newTestRenameServer(t, tmp)
		code, resp := callRename(t, s, src, bad)
		if code != http.StatusBadRequest || resp.OK {
			t.Fatalf("expected error for %q, got code=%d resp=%+v", bad, code, resp)
		}
	}
}

func TestFilesRename_RejectOutsideAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()
	src := filepath.Join(outside, "foo.md")
	_ = os.WriteFile(src, []byte("x"), 0o644)

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "bar.md")
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesRename_RejectMissingToken(t *testing.T) {
	tmp := t.TempDir()
	s := newTestRenameServer(t, tmp)
	body, _ := json.Marshal(filesRenameReq{Src: filepath.Join(tmp, "x"), NewName: "y"})
	req := httptest.NewRequest(http.MethodPost, "/api/files-rename", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesRename(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

func TestFilesRename_RejectNonAbsolute(t *testing.T) {
	tmp := t.TempDir()
	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, "foo.md", "bar.md")
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" || !strings.Contains(resp.Detail, "absolute") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

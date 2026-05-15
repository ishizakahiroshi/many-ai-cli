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
	w := httptest.NewRecorder()
	s.handleFilesRename(w, req)
	var resp filesRenameResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesRename_OK_File(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "old.md")
	if err := os.WriteFile(src, []byte("hi"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "new.md")
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	want := filepath.Join(tmp, "new.md")
	if resp.NewAbs != want {
		t.Fatalf("newAbs = %q, want %q", resp.NewAbs, want)
	}
	if _, err := os.Stat(src); !os.IsNotExist(err) {
		t.Fatalf("src should be gone: %v", err)
	}
	if _, err := os.Stat(want); err != nil {
		t.Fatalf("dst should exist: %v", err)
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
	src := filepath.Join(tmp, "foo.md")
	_ = os.WriteFile(src, []byte("x"), 0o644)

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "foo.md")
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "identical") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesRename_RejectOverwrite(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "old.md")
	dst := filepath.Join(tmp, "new.md")
	_ = os.WriteFile(src, []byte("src"), 0o644)
	_ = os.WriteFile(dst, []byte("dst"), 0o644)

	s := newTestRenameServer(t, tmp)
	code, resp := callRename(t, s, src, "new.md")
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "already exists") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesRename_RejectSeparatorInName(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "foo.md")
	_ = os.WriteFile(src, []byte("x"), 0o644)

	for _, bad := range []string{"a/b.md", `a\b.md`, "..", ".", "", "C:bad"} {
		s := newTestRenameServer(t, tmp)
		code, resp := callRename(t, s, src, bad)
		if code != http.StatusOK || resp.OK {
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
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "forbidden") {
		t.Fatalf("unexpected error: %q", resp.Error)
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
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "absolute") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

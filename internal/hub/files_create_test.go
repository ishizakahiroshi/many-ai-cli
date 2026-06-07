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

func newTestCreateServer(t *testing.T, tmpDir string) *Server {
	t.Helper()
	cfg := &config.Config{Token: "tok"}
	s := &Server{
		cfg:      cfg,
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
	return s
}

func callCreate(t *testing.T, s *Server, dir, name string) (int, filesCreateResp) {
	t.Helper()
	body, _ := json.Marshal(filesCreateReq{Dir: dir, Name: name})
	req := httptest.NewRequest(http.MethodPost, "/api/files-create?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesCreate(w, req)
	var resp filesCreateResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesCreate_OK(t *testing.T) {
	tmp := t.TempDir()
	s := newTestCreateServer(t, tmp)

	code, resp := callCreate(t, s, tmp, "new.txt")
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	want := filepath.Join(tmp, "new.txt")
	if resp.NewAbs != want {
		t.Fatalf("newAbs = %q, want %q", resp.NewAbs, want)
	}
	info, err := os.Stat(want)
	if err != nil {
		t.Fatalf("file not created: %v", err)
	}
	if info.IsDir() {
		t.Fatal("created entry is a directory")
	}
	if info.Size() != 0 {
		t.Fatalf("created file size = %d, want 0", info.Size())
	}
}

func TestFilesCreate_RejectOutsideAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()

	s := newTestCreateServer(t, tmp)
	code, resp := callCreate(t, s, outside, "new.txt")
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected forbidden, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error code: %q", resp.Error)
	}
}

func TestFilesCreate_RejectAlreadyExists(t *testing.T) {
	tmp := t.TempDir()
	existing := filepath.Join(tmp, "existing.txt")
	if err := os.WriteFile(existing, []byte("keep"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestCreateServer(t, tmp)
	code, resp := callCreate(t, s, tmp, "existing.txt")
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected conflict, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "already_exists" {
		t.Fatalf("unexpected error code: %q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesCreate_RejectNonTextFile(t *testing.T) {
	tmp := t.TempDir()
	s := newTestCreateServer(t, tmp)

	code, resp := callCreate(t, s, tmp, "image.png")
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected forbidden, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "not_previewable" {
		t.Fatalf("unexpected error code: %q", resp.Error)
	}
	if _, err := os.Stat(filepath.Join(tmp, "image.png")); !os.IsNotExist(err) {
		t.Fatalf("non-text file should not be created, stat err=%v", err)
	}
}

func TestFilesCreate_RejectSeparatorInName(t *testing.T) {
	tmp := t.TempDir()
	s := newTestCreateServer(t, tmp)

	for _, bad := range []string{"a/b.txt", `a\b.txt`, "..", ".", "", "C:bad.txt"} {
		code, resp := callCreate(t, s, tmp, bad)
		if code != http.StatusBadRequest || resp.OK {
			t.Fatalf("expected bad_request for %q, got code=%d resp=%+v", bad, code, resp)
		}
	}
}

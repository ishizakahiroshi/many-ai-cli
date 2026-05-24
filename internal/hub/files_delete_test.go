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
)

func callDeleteDir(t *testing.T, s *Server, src string) (int, filesDeleteDirResp) {
	t.Helper()
	body, _ := json.Marshal(filesDeleteDirReq{Src: src})
	req := httptest.NewRequest(http.MethodPost, "/api/files-delete-dir?token=tok", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesDeleteDir(w, req)
	var resp filesDeleteDirResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesDeleteDir_OK_Directory(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "delete-me")
	if err := os.MkdirAll(filepath.Join(src, "child"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(src, "child", "file.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callDeleteDir(t, s, src)
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	if _, err := os.Stat(src); !os.IsNotExist(err) {
		t.Fatalf("src should be gone: %v", err)
	}
}

func TestFilesDeleteDir_RejectFile(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "file.txt")
	if err := os.WriteFile(src, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callDeleteDir(t, s, src)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "directory") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesDeleteDir_RejectAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	s := newTestRenameServer(t, tmp)
	code, resp := callDeleteDir(t, s, tmp)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "allowed root") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesDeleteDir_RejectOutsideAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()
	src := filepath.Join(outside, "dir")
	if err := os.MkdirAll(src, 0o755); err != nil {
		t.Fatal(err)
	}

	s := newTestRenameServer(t, tmp)
	code, resp := callDeleteDir(t, s, src)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "forbidden") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

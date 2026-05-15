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

// newTestMoveServer は handleFilesMove テスト用に最小限の Server を組み立てる。
// hubCWD = tmpDir、token = "tok" 固定。
func newTestMoveServer(t *testing.T, tmpDir string) *Server {
	t.Helper()
	cfg := &config.Config{Token: "tok"}
	s := &Server{
		cfg:      cfg,
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
	return s
}

// callMove は /api/files-move を POST し、レスポンス body を返す。
func callMove(t *testing.T, s *Server, src, dstDir string) (int, filesMoveResp) {
	t.Helper()
	body, _ := json.Marshal(filesMoveReq{Src: src, DstDir: dstDir})
	req := httptest.NewRequest(http.MethodPost, "/api/files-move?token=tok", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesMove(w, req)
	var resp filesMoveResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

func TestFilesMove_OK_FileToSubdir(t *testing.T) {
	tmp := t.TempDir()
	subA := filepath.Join(tmp, "a")
	subB := filepath.Join(tmp, "b")
	if err := os.MkdirAll(subA, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(subB, 0o755); err != nil {
		t.Fatal(err)
	}
	src := filepath.Join(subA, "foo.md")
	if err := os.WriteFile(src, []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, src, subB)
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	want := filepath.Join(subB, "foo.md")
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

func TestFilesMove_RejectSameDir(t *testing.T) {
	tmp := t.TempDir()
	src := filepath.Join(tmp, "foo.md")
	if err := os.WriteFile(src, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, src, tmp)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "already in") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesMove_RejectOverwrite(t *testing.T) {
	tmp := t.TempDir()
	subA := filepath.Join(tmp, "a")
	subB := filepath.Join(tmp, "b")
	_ = os.MkdirAll(subA, 0o755)
	_ = os.MkdirAll(subB, 0o755)
	src := filepath.Join(subA, "foo.md")
	dst := filepath.Join(subB, "foo.md")
	_ = os.WriteFile(src, []byte("src"), 0o644)
	_ = os.WriteFile(dst, []byte("dst"), 0o644)

	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, src, subB)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "already exists") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesMove_RejectDirIntoDescendant(t *testing.T) {
	tmp := t.TempDir()
	parent := filepath.Join(tmp, "parent")
	child := filepath.Join(parent, "child")
	_ = os.MkdirAll(child, 0o755)

	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, parent, child)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "into itself") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesMove_RejectOutsideAllowedRoot(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir() // 別の独立した temp dir = 許可ルート外
	src := filepath.Join(tmp, "foo.md")
	_ = os.WriteFile(src, []byte("x"), 0o644)
	dstDir := filepath.Join(outside, "sub")
	_ = os.MkdirAll(dstDir, 0o755)

	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, src, dstDir)
	if code != http.StatusOK || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if !strings.Contains(resp.Error, "forbidden") {
		t.Fatalf("unexpected error: %q", resp.Error)
	}
}

func TestFilesMove_RejectMissingToken(t *testing.T) {
	tmp := t.TempDir()
	s := newTestMoveServer(t, tmp)
	body, _ := json.Marshal(filesMoveReq{Src: filepath.Join(tmp, "x"), DstDir: tmp})
	req := httptest.NewRequest(http.MethodPost, "/api/files-move", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesMove(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

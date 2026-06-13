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

	"many-ai-cli/internal/config"
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
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesMove(w, req)
	var resp filesMoveResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

// callMoveMulti は /api/files-move を多ファイルモード（Srcs フィールド）で POST する。
func callMoveMulti(t *testing.T, s *Server, srcs []string, dstDir string) (int, filesMoveResp) {
	t.Helper()
	body, _ := json.Marshal(filesMoveReq{Srcs: srcs, DstDir: dstDir})
	req := httptest.NewRequest(http.MethodPost, "/api/files-move?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
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
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" || !strings.Contains(resp.Detail, "already in") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
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
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" || !strings.Contains(resp.Detail, "already exists") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

func TestFilesMove_RejectDirIntoDescendant(t *testing.T) {
	tmp := t.TempDir()
	parent := filepath.Join(tmp, "parent")
	child := filepath.Join(parent, "child")
	_ = os.MkdirAll(child, 0o755)

	s := newTestMoveServer(t, tmp)
	code, resp := callMove(t, s, parent, child)
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" || !strings.Contains(resp.Detail, "into itself") {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
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
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected error, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
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

func TestFilesMove_Multi_AllOK(t *testing.T) {
	tmp := t.TempDir()
	subA := filepath.Join(tmp, "a")
	subB := filepath.Join(tmp, "b")
	_ = os.MkdirAll(subA, 0o755)
	_ = os.MkdirAll(subB, 0o755)
	f1 := filepath.Join(subA, "foo.md")
	f2 := filepath.Join(subA, "bar.txt")
	_ = os.WriteFile(f1, []byte("foo"), 0o644)
	_ = os.WriteFile(f2, []byte("bar"), 0o644)

	s := newTestMoveServer(t, tmp)
	code, resp := callMoveMulti(t, s, []string{f1, f2}, subB)
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	if len(resp.Results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(resp.Results))
	}
	for _, r := range resp.Results {
		if r.Error != "" {
			t.Fatalf("unexpected error for %s: %s", r.Src, r.Error)
		}
		if _, err := os.Stat(r.NewAbs); err != nil {
			t.Fatalf("dst should exist: %v", err)
		}
	}
}

func TestFilesMove_Multi_PreflightFailureDoesNotMoveAnyFile(t *testing.T) {
	tmp := t.TempDir()
	subA := filepath.Join(tmp, "a")
	subB := filepath.Join(tmp, "b")
	_ = os.MkdirAll(subA, 0o755)
	_ = os.MkdirAll(subB, 0o755)
	f1 := filepath.Join(subA, "foo.md")
	_ = os.WriteFile(f1, []byte("foo"), 0o644)
	f2 := filepath.Join(subA, "notexist.md") // 存在しない → エラー

	s := newTestMoveServer(t, tmp)
	code, resp := callMoveMulti(t, s, []string{f1, f2}, subB)
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected partial-fail (ok=false), got code=%d resp=%+v", code, resp)
	}
	if len(resp.Results) != 2 {
		t.Fatalf("expected 2 results, got %d", len(resp.Results))
	}
	if resp.Results[0].Error != "" || resp.Results[0].NewAbs != "" {
		t.Fatalf("expected f1 to remain pending with no move result: %+v", resp.Results[0])
	}
	if resp.Results[1].Error == "" {
		t.Fatalf("expected f2 to fail")
	}
	if _, err := os.Stat(f1); err != nil {
		t.Fatalf("f1 should remain at source after preflight failure: %v", err)
	}
	if _, err := os.Stat(filepath.Join(subB, "foo.md")); !os.IsNotExist(err) {
		t.Fatalf("f1 should not be moved after preflight failure: %v", err)
	}
}

func TestFilesMove_Multi_RejectDuplicateTargetBeforeMoving(t *testing.T) {
	tmp := t.TempDir()
	subA := filepath.Join(tmp, "a")
	subB := filepath.Join(tmp, "b")
	subC := filepath.Join(tmp, "c")
	_ = os.MkdirAll(subA, 0o755)
	_ = os.MkdirAll(subB, 0o755)
	_ = os.MkdirAll(subC, 0o755)
	f1 := filepath.Join(subA, "same.md")
	f2 := filepath.Join(subC, "same.md")
	_ = os.WriteFile(f1, []byte("one"), 0o644)
	_ = os.WriteFile(f2, []byte("two"), 0o644)

	s := newTestMoveServer(t, tmp)
	code, resp := callMoveMulti(t, s, []string{f1, f2}, subB)
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected duplicate-target failure, got code=%d resp=%+v", code, resp)
	}
	if len(resp.Results) != 2 || resp.Results[0].Error == "" || resp.Results[1].Error == "" {
		t.Fatalf("expected both duplicate-target results to contain errors: %+v", resp.Results)
	}
	if _, err := os.Stat(f1); err != nil {
		t.Fatalf("f1 should remain at source: %v", err)
	}
	if _, err := os.Stat(f2); err != nil {
		t.Fatalf("f2 should remain at source: %v", err)
	}
	if _, err := os.Stat(filepath.Join(subB, "same.md")); !os.IsNotExist(err) {
		t.Fatalf("target should not be created: %v", err)
	}
}

func TestFilesMove_Multi_RejectAncestorAndDescendantBeforeMoving(t *testing.T) {
	tmp := t.TempDir()
	subB := filepath.Join(tmp, "b")
	parent := filepath.Join(tmp, "parent")
	child := filepath.Join(parent, "child.txt")
	_ = os.MkdirAll(parent, 0o755)
	_ = os.MkdirAll(subB, 0o755)
	_ = os.WriteFile(child, []byte("x"), 0o644)

	s := newTestMoveServer(t, tmp)
	code, resp := callMoveMulti(t, s, []string{parent, child}, subB)
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected ancestor-descendant failure, got code=%d resp=%+v", code, resp)
	}
	if len(resp.Results) != 2 || resp.Results[0].Error == "" || resp.Results[1].Error == "" {
		t.Fatalf("expected both ancestor-descendant results to contain errors: %+v", resp.Results)
	}
	if _, err := os.Stat(parent); err != nil {
		t.Fatalf("parent should remain at source: %v", err)
	}
	if _, err := os.Stat(child); err != nil {
		t.Fatalf("child should remain at source: %v", err)
	}
}

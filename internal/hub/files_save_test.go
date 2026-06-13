package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

// newTestSaveServer はテスト用 Server を生成する（files_rename_test.go のヘルパ構成に準拠）。
func newTestSaveServer(t *testing.T, tmpDir string) *Server {
	t.Helper()
	cfg := &config.Config{Token: "tok"}
	s := &Server{
		cfg:      cfg,
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
	return s
}

// callSave は POST /api/files-save を呼び出し、ステータスコードとレスポンスを返す。
func callSave(t *testing.T, s *Server, path, content string, baseMtime time.Time) (int, filesSaveResp) {
	t.Helper()
	body, _ := json.Marshal(filesSaveReq{Path: path, Content: content, BaseMtime: baseMtime})
	req := httptest.NewRequest(http.MethodPost, "/api/files-save?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesSave(w, req)
	var resp filesSaveResp
	_ = json.NewDecoder(w.Body).Decode(&resp)
	return w.Code, resp
}

// TestFilesSave_OK は正常系（保存成功・mtime 更新・内容反映）を確認する。
func TestFilesSave_OK(t *testing.T) {
	tmp := t.TempDir()
	target := filepath.Join(tmp, "hello.txt")
	if err := os.WriteFile(target, []byte("original"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, target, "updated content", time.Time{})
	if code != http.StatusOK || !resp.OK {
		t.Fatalf("expected ok, got code=%d resp=%+v", code, resp)
	}
	if resp.Path != target {
		t.Fatalf("path = %q, want %q", resp.Path, target)
	}
	if resp.Mtime.IsZero() {
		t.Fatal("mtime should not be zero")
	}

	// 内容が実際に書き込まれたことを確認
	got, _ := os.ReadFile(target)
	if string(got) != "updated content" {
		t.Fatalf("file content = %q, want %q", string(got), "updated content")
	}
}

// TestFilesSave_MissingToken は token なしで 401 を返すことを確認する。
func TestFilesSave_MissingToken(t *testing.T) {
	tmp := t.TempDir()
	s := newTestSaveServer(t, tmp)
	body, _ := json.Marshal(filesSaveReq{Path: filepath.Join(tmp, "a.txt"), Content: "x"})
	req := httptest.NewRequest(http.MethodPost, "/api/files-save", bytes.NewReader(body))
	w := httptest.NewRecorder()
	s.handleFilesSave(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", w.Code)
	}
}

// TestFilesSave_WrongMethod は GET で 405 を返すことを確認する。
func TestFilesSave_WrongMethod(t *testing.T) {
	tmp := t.TempDir()
	s := newTestSaveServer(t, tmp)
	req := httptest.NewRequest(http.MethodGet, "/api/files-save?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleFilesSave(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Fatalf("expected 405, got %d", w.Code)
	}
}

// TestFilesSave_RelativePath は相対パスで 400 を返すことを確認する。
func TestFilesSave_RelativePath(t *testing.T) {
	tmp := t.TempDir()
	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, "relative/path.txt", "content", time.Time{})
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected 400, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

// TestFilesSave_OutsideScope はスコープ外のパスで 403 を返すことを確認する。
func TestFilesSave_OutsideScope(t *testing.T) {
	tmp := t.TempDir()
	outside := t.TempDir()
	target := filepath.Join(outside, "secret.txt")
	_ = os.WriteFile(target, []byte("x"), 0o644)

	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, target, "hacked", time.Time{})
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected 403, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

// TestFilesSave_NonTextExtension は非テキスト拡張子で 403 を返すことを確認する。
func TestFilesSave_NonTextExtension(t *testing.T) {
	tmp := t.TempDir()
	target := filepath.Join(tmp, "image.png")
	_ = os.WriteFile(target, []byte("fake png"), 0o644)

	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, target, "new content", time.Time{})
	if code != http.StatusForbidden || resp.OK {
		t.Fatalf("expected 403, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "forbidden" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

// TestFilesSave_SizeExceeded はサイズ超過で 413 を返すことを確認する。
func TestFilesSave_SizeExceeded(t *testing.T) {
	tmp := t.TempDir()
	target := filepath.Join(tmp, "big.txt")
	_ = os.WriteFile(target, []byte("original"), 0o644)

	s := newTestSaveServer(t, tmp)
	// filesContentMaxSize + 1 バイトの文字列を生成（content フィールド単体でも 1 MiB 超）
	huge := string(make([]byte, filesContentMaxSize+1))
	code, resp := callSave(t, s, target, huge, time.Time{})
	if code != http.StatusRequestEntityTooLarge || resp.OK {
		t.Fatalf("expected 413, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "too_large" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

// TestFilesSave_BaseMtimeMismatch は baseMtime 不一致で 409 を返し、
// レスポンスに現在の mtime が含まれることを確認する。
func TestFilesSave_BaseMtimeMismatch(t *testing.T) {
	tmp := t.TempDir()
	target := filepath.Join(tmp, "edit.txt")
	_ = os.WriteFile(target, []byte("original"), 0o644)

	// 古い mtime（1 時間前）を baseMtime として送信
	oldMtime := time.Now().Add(-1 * time.Hour)

	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, target, "new content", oldMtime)
	if code != http.StatusConflict || resp.OK {
		t.Fatalf("expected 409, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "conflict" {
		t.Fatalf("unexpected error code: %q", resp.Error)
	}
	// レスポンスに現在の mtime が含まれること
	if resp.Mtime.IsZero() {
		t.Fatal("conflict response should include current mtime")
	}
}

// TestFilesSave_FileNotFound は存在しないファイルで 404 を返すことを確認する。
func TestFilesSave_FileNotFound(t *testing.T) {
	tmp := t.TempDir()
	target := filepath.Join(tmp, "nonexistent.txt")

	s := newTestSaveServer(t, tmp)
	code, resp := callSave(t, s, target, "content", time.Time{})
	if code != http.StatusNotFound || resp.OK {
		t.Fatalf("expected 404, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "not_found" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

// TestFilesSave_DirectoryPath はディレクトリを指定した場合に 400 を返すことを確認する。
func TestFilesSave_DirectoryPath(t *testing.T) {
	tmp := t.TempDir()
	dir := filepath.Join(tmp, "subdir")
	// ディレクトリ名に .txt をつけて isTextFile チェックを通過させる
	// （実際は Stat でディレクトリ判定が先に来るため）
	_ = os.MkdirAll(dir, 0o755)

	s := newTestSaveServer(t, tmp)
	// ディレクトリ自体を path に指定（拡張子なし → forbidden になる可能性があるため .go 相当の名前にする）
	// files_save.go では isTextFile → stat の順で判定するため、
	// ディレクトリにテキスト拡張子を付けた名前で確認する
	dirWithExt := filepath.Join(tmp, "fakefile.txt")
	_ = os.MkdirAll(dirWithExt, 0o755)

	code, resp := callSave(t, s, dirWithExt, "content", time.Time{})
	if code != http.StatusBadRequest || resp.OK {
		t.Fatalf("expected 400 for directory path, got code=%d resp=%+v", code, resp)
	}
	if resp.Error != "bad_request" {
		t.Fatalf("unexpected error: code=%q detail=%q", resp.Error, resp.Detail)
	}
}

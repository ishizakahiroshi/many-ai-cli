package hub

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func memoTestServer(t *testing.T) *Server {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "memo-test-token"
	s.memos = newMemoManager(filepath.Join(t.TempDir(), "memos.json"))
	return s
}

func memoRequest(s *Server, method, path string, body any) *httptest.ResponseRecorder {
	encoded, _ := json.Marshal(body)
	r := prefsAuthReq(method, path, encoded, "application/json")
	w := httptest.NewRecorder()
	s.handleMemos(w, r)
	return w
}

func TestMemoCreateListUpdateDeleteRoundTrip(t *testing.T) {
	s := memoTestServer(t)
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": "  次: plan の C8 から再開  "})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.Memo.Text != "次: plan の C8 から再開" || created.Memo.Project != "" || created.Memo.Done {
		t.Fatalf("create ignored trimming or defaults: %+v", created.Memo)
	}
	if created.Memo.ID == "" || created.Memo.CreatedAt == "" || created.Memo.UpdatedAt == "" {
		t.Fatalf("create left required fields empty: %+v", created.Memo)
	}

	w = memoRequest(s, http.MethodGet, "/api/memos?token=memo-test-token", nil)
	if w.Code != 200 || !strings.Contains(w.Body.String(), created.Memo.ID) {
		t.Fatalf("list=%d %s", w.Code, w.Body.String())
	}

	w = memoRequest(s, http.MethodPatch, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", map[string]any{"done": true})
	if w.Code != 200 {
		t.Fatalf("done=%d %s", w.Code, w.Body.String())
	}
	var doneResp struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &doneResp); err != nil {
		t.Fatal(err)
	}
	if !doneResp.Memo.Done || doneResp.Memo.DoneAt == "" {
		t.Fatalf("done not recorded: %+v", doneResp.Memo)
	}

	w = memoRequest(s, http.MethodPatch, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", map[string]any{"text": "編集後の本文"})
	if w.Code != 200 {
		t.Fatalf("edit=%d %s", w.Code, w.Body.String())
	}
	var editResp struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &editResp); err != nil {
		t.Fatal(err)
	}
	if editResp.Memo.Text != "編集後の本文" || !editResp.Memo.Done {
		t.Fatalf("edit lost done flag: %+v", editResp.Memo)
	}

	w = memoRequest(s, http.MethodDelete, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", nil)
	if w.Code != 200 {
		t.Fatalf("delete=%d %s", w.Code, w.Body.String())
	}
	w = memoRequest(s, http.MethodGet, "/api/memos?token=memo-test-token", nil)
	if strings.Contains(w.Body.String(), created.Memo.ID) {
		t.Fatal("deleted memo still listed")
	}
}

func TestMemoCreateWithSessionIDUsesGitRoot(t *testing.T) {
	s := memoTestServer(t)
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, ".git"), 0o755); err != nil {
		t.Fatal(err)
	}
	sub := filepath.Join(root, "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	s.sessions[7] = &session{ID: 7, CWD: sub}
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "next", "session_id": 7})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.Memo.Project != root {
		t.Fatalf("project = %q, want %q", created.Memo.Project, root)
	}
}

func TestMemoCreateWithoutSessionIDHasEmptyProject(t *testing.T) {
	s := memoTestServer(t)
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "next"})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.Memo.Project != "" {
		t.Fatalf("project = %q, want empty", created.Memo.Project)
	}
}

func TestMemoHTTPAuthAndValidation(t *testing.T) {
	s := memoTestServer(t)
	if w := memoRequest(s, http.MethodGet, "/api/memos", nil); w.Code != 401 {
		t.Fatalf("unauthenticated status=%d", w.Code)
	}
	if w := memoRequest(s, http.MethodGet, "/api/memos?token=memo-test-token", nil); w.Code != 200 {
		t.Fatalf("list status=%d", w.Code)
	}
	if w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": ""}); w.Code != 400 {
		t.Fatalf("empty text status=%d", w.Code)
	}
	if w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": strings.Repeat("a", memoTextMaxLen+1)}); w.Code != 400 {
		t.Fatalf("too long status=%d", w.Code)
	}
	if w := memoRequest(s, http.MethodPatch, "/api/memos/does-not-exist?token=memo-test-token", map[string]any{"done": true}); w.Code != 404 {
		t.Fatalf("missing patch status=%d", w.Code)
	}
	if w := memoRequest(s, http.MethodDelete, "/api/memos/does-not-exist?token=memo-test-token", nil); w.Code != 404 {
		t.Fatalf("missing delete status=%d", w.Code)
	}
}

func TestMemoCountLimitRejectsBeyond500(t *testing.T) {
	s := memoTestServer(t)
	s.memos.mu.Lock()
	seeded := make([]memo, memoMaxCount)
	for i := range seeded {
		seeded[i] = memo{ID: fmt.Sprintf("seed-%d", i), Text: "x", CreatedAt: "2026-01-01T00:00:00Z", UpdatedAt: "2026-01-01T00:00:00Z"}
	}
	s.memos.data.Memos = seeded
	s.memos.mu.Unlock()
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": "over limit"})
	if w.Code != 400 {
		t.Fatalf("limit status=%d %s", w.Code, w.Body.String())
	}
}

func TestMemoMentionsReturnsTextAndProject(t *testing.T) {
	s := memoTestServer(t)
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": "path note"})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	got := s.memoMentions()
	if len(got) != 1 || got[0].Text != "path note" || got[0].Project != "" {
		t.Fatalf("memoMentions = %+v", got)
	}
}

func TestMemoPersistsAcrossManagerReload(t *testing.T) {
	s := memoTestServer(t)
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]string{"text": "persisted"})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	reloaded := newMemoManager(s.memos.path)
	if reloaded.loadErr != nil || len(reloaded.data.Memos) != 1 || reloaded.data.Memos[0].ID != created.Memo.ID {
		t.Fatalf("reload = %+v, err=%v", reloaded.data, reloaded.loadErr)
	}
}

func TestMemoCorruptStoreReturns503WithoutOverwriting(t *testing.T) {
	s := memoTestServer(t)
	if err := os.WriteFile(s.memos.path, []byte("not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.memos = newMemoManager(s.memos.path)
	w := memoRequest(s, http.MethodGet, "/api/memos?token=memo-test-token", nil)
	if w.Code != 503 {
		t.Fatalf("corrupt status=%d", w.Code)
	}
	b, _ := os.ReadFile(s.memos.path)
	if string(b) != "not json" {
		t.Fatal("corrupt file overwritten")
	}
}

package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// PNG のシグネチャだけを持つ合成データ（http.DetectContentType は先頭 8 バイトで判定する）。
var memoTestPNG = append([]byte("\x89PNG\r\n\x1a\n"), make([]byte, 32)...)

func memoImageRequest(s *Server, method, path string, body []byte) *httptest.ResponseRecorder {
	r := prefsAuthReq(method, path, body, "image/png")
	w := httptest.NewRecorder()
	s.handleMemoImages(w, r)
	return w
}

func uploadMemoTestImage(t *testing.T, s *Server) string {
	t.Helper()
	w := memoImageRequest(s, http.MethodPost, "/api/memo-images?token=memo-test-token", memoTestPNG)
	if w.Code != 200 {
		t.Fatalf("upload=%d %s", w.Code, w.Body.String())
	}
	var resp struct {
		Image string `json:"image"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if !memoImageNameRe.MatchString(resp.Image) {
		t.Fatalf("image name = %q", resp.Image)
	}
	return resp.Image
}

func TestMemoImageOnlyMemoRoundTripRemovesFileOnDelete(t *testing.T) {
	s := memoTestServer(t)
	name := uploadMemoTestImage(t, s)

	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "", "images": []string{name}})
	if w.Code != 200 {
		t.Fatalf("create=%d %s", w.Code, w.Body.String())
	}
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if len(created.Memo.Images) != 1 || created.Memo.Images[0] != name {
		t.Fatalf("images = %v", created.Memo.Images)
	}

	w = memoImageRequest(s, http.MethodGet, "/api/memo-images/"+name+"?token=memo-test-token", nil)
	if w.Code != 200 || w.Header().Get("Content-Type") != "image/png" || w.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatalf("get=%d ct=%q", w.Code, w.Header().Get("Content-Type"))
	}

	// 画像だけのメモは本文を空にしたままでよい。
	w = memoRequest(s, http.MethodPatch, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", map[string]any{"text": ""})
	if w.Code != 200 {
		t.Fatalf("empty text patch on image memo=%d %s", w.Code, w.Body.String())
	}

	w = memoRequest(s, http.MethodDelete, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", nil)
	if w.Code != 200 {
		t.Fatalf("delete=%d %s", w.Code, w.Body.String())
	}
	if _, err := os.Stat(filepath.Join(s.memos.imagesDir, name)); !os.IsNotExist(err) {
		t.Fatalf("image file survived memo delete: %v", err)
	}
}

func TestMemoImageUploadRejectsNonImage(t *testing.T) {
	s := memoTestServer(t)
	w := memoImageRequest(s, http.MethodPost, "/api/memo-images?token=memo-test-token", []byte("<svg xmlns='http://www.w3.org/2000/svg'></svg>"))
	if w.Code != http.StatusUnsupportedMediaType {
		t.Fatalf("status=%d %s", w.Code, w.Body.String())
	}
	if w := memoImageRequest(s, http.MethodPost, "/api/memo-images", memoTestPNG); w.Code != 401 {
		t.Fatalf("unauthenticated status=%d", w.Code)
	}
}

func TestMemoImageReferencesAreValidated(t *testing.T) {
	s := memoTestServer(t)
	name := uploadMemoTestImage(t, s)
	bad := [][]string{
		{"../memos.json"},
		{"0123456789abcdef0123456789abcdef.png"}, // 形は正しいがファイルが無い
		{name, name},
	}
	for _, images := range bad {
		if w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "x", "images": images}); w.Code != 400 {
			t.Fatalf("images=%v status=%d", images, w.Code)
		}
	}
	if w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "first", "images": []string{name}}); w.Code != 200 {
		t.Fatalf("first=%d %s", w.Code, w.Body.String())
	}
	// 同じ画像を 2 つのメモに付けさせない（片方の削除でもう片方の画像が消えるため）。
	if w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "second", "images": []string{name}}); w.Code != 400 {
		t.Fatalf("shared image status=%d", w.Code)
	}
	if w := memoImageRequest(s, http.MethodGet, "/api/memo-images/..%2Fmemos.json?token=memo-test-token", nil); w.Code != 404 {
		t.Fatalf("traversal get status=%d", w.Code)
	}
}

func TestMemoEmptyTextStillRejectedWithoutImages(t *testing.T) {
	s := memoTestServer(t)
	w := memoRequest(s, http.MethodPost, "/api/memos?token=memo-test-token", map[string]any{"text": "text only"})
	var created struct {
		Memo memo `json:"memo"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if w := memoRequest(s, http.MethodPatch, "/api/memos/"+created.Memo.ID+"?token=memo-test-token", map[string]any{"text": "  "}); w.Code != 400 {
		t.Fatalf("empty text patch on text memo=%d", w.Code)
	}
}

func TestMemoCleanOrphanImagesKeepsReferencedAndRecent(t *testing.T) {
	s := memoTestServer(t)
	dir := s.memos.imagesDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	write := func(name string, age time.Duration) {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, memoTestPNG, 0o600); err != nil {
			t.Fatal(err)
		}
		old := time.Now().Add(-age)
		if err := os.Chtimes(path, old, old); err != nil {
			t.Fatal(err)
		}
	}
	referenced := "11111111111111111111111111111111.png"
	orphanOld := "22222222222222222222222222222222.png"
	orphanNew := "33333333333333333333333333333333.png"
	unrelated := "keep-me.txt"
	write(referenced, 48*time.Hour)
	write(orphanOld, 48*time.Hour)
	write(orphanNew, time.Minute)
	write(unrelated, 48*time.Hour)
	s.memos.data.Memos = []memo{{ID: "m1", Images: []string{referenced}}}

	if n := s.memos.cleanOrphanImages(time.Now()); n != 1 {
		t.Fatalf("removed = %d, want 1", n)
	}
	for name, want := range map[string]bool{referenced: true, orphanOld: false, orphanNew: true, unrelated: true} {
		_, err := os.Stat(filepath.Join(dir, name))
		if (err == nil) != want {
			t.Fatalf("%s exists=%v, want %v", name, err == nil, want)
		}
	}

	// memos.json を読めていないときは参照の有無が分からないので何も消さない。
	write(orphanOld, 48*time.Hour)
	s.memos.loadErr = os.ErrInvalid
	if n := s.memos.cleanOrphanImages(time.Now()); n != 0 {
		t.Fatalf("removed with loadErr = %d", n)
	}
}

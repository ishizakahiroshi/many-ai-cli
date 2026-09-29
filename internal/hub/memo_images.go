package hub

import (
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// 作業メモに貼る画像（スクリーンショット等）。
//
// 置き場所は ~/.many-ai-cli/memo-images/（memos.json の隣）。セッションへの添付
// （~/.many-ai-cli/attachments）は保持日数・合計サイズの上限・設定画面の一括削除で
// 消えるので、そこへ置くと「メモは残っているのに画像だけ消える」ことになる。
// メモには画像名（ランダム hex + 拡張子）だけを持たせ、パスは持たせない。
//
// 画像ファイルが消えるのは次の 2 つだけ:
//  1. その画像を持つメモを削除したとき（handleMemoDelete）
//  2. 貼り付けたがメモに付けずに終わった画像を、次回起動時に回収するとき（cleanOrphanImages）
//
// ログ・添付のローテーション（maintenance.go）と設定画面の一括削除（purge_handlers.go）は
// このフォルダを対象にしない。容量は「古い順に消す」ローテーションではなく、合計が
// memoImagesTotalMaxBytes を超える貼り付けを断ることで抑える（古い順に消すと、残っている
// メモから画像だけが消える）。
const (
	memoImageMaxBytes       = 10 * 1024 * 1024
	memoImagesTotalMaxBytes = 500 * 1024 * 1024
	memoImagesPerMemo       = 10
	// 貼り付けてからメモを追加するまでの猶予。これより古く、どのメモからも参照されない画像を回収する。
	memoImageOrphanAge = 24 * time.Hour
)

var memoImageNameRe = regexp.MustCompile(`^[0-9a-f]{32}\.(png|jpg|gif|webp)$`)

// MIME は client の Content-Type を信用せず sniff した値で決める（アバター画像と同じ方針・SVG は受けない）。
var memoImageExtByMime = map[string]string{
	"image/png":  "png",
	"image/jpeg": "jpg",
	"image/gif":  "gif",
	"image/webp": "webp",
}

func memoImageMime(name string) string {
	ext := strings.TrimPrefix(filepath.Ext(name), ".")
	for mime, e := range memoImageExtByMime {
		if e == ext {
			return mime
		}
	}
	return "application/octet-stream"
}

// validateImagesLocked はメモに付ける画像名を検証する。exceptID のメモ自身が持つ画像は
// 「他のメモが使用中」とみなさない。1 つの画像を 2 つのメモで共有させない（片方の削除で
// もう片方の画像まで消えるため）。m.mu を保持したまま呼ぶこと。
func (m *memoManager) validateImagesLocked(names []string, exceptID string) error {
	if len(names) > memoImagesPerMemo {
		return errors.New("a memo can hold up to 10 images")
	}
	used := map[string]bool{}
	for _, item := range m.data.Memos {
		if item.ID == exceptID {
			continue
		}
		for _, name := range item.Images {
			used[name] = true
		}
	}
	seen := map[string]bool{}
	for _, name := range names {
		if !memoImageNameRe.MatchString(name) || seen[name] || used[name] {
			return errors.New("invalid image reference")
		}
		seen[name] = true
		if _, err := os.Stat(filepath.Join(m.imagesDir, name)); err != nil {
			return errors.New("image not found")
		}
	}
	return nil
}

func (m *memoManager) removeImages(names []string) []error {
	var errs []error
	for _, name := range names {
		if !memoImageNameRe.MatchString(name) {
			continue
		}
		if err := os.Remove(filepath.Join(m.imagesDir, name)); err != nil && !errors.Is(err, os.ErrNotExist) {
			errs = append(errs, err)
		}
	}
	return errs
}

// cleanOrphanImages は、どのメモからも参照されず、memoImageOrphanAge より古い画像を消す。
// memos.json を読めていないとき（loadErr）は参照の有無が分からないので何も消さない。
// 自分が作った名前の形に合わないファイルには触らない。
func (m *memoManager) cleanOrphanImages(now time.Time) (removed int) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.loadErr != nil || m.imagesDir == "" {
		return 0
	}
	referenced := map[string]bool{}
	for _, item := range m.data.Memos {
		for _, name := range item.Images {
			referenced[name] = true
		}
	}
	entries, err := os.ReadDir(m.imagesDir)
	if err != nil {
		return 0
	}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !memoImageNameRe.MatchString(name) || referenced[name] {
			continue
		}
		info, err := e.Info()
		if err != nil || now.Sub(info.ModTime()) < memoImageOrphanAge {
			continue
		}
		if os.Remove(filepath.Join(m.imagesDir, name)) == nil {
			removed++
		}
	}
	return removed
}

// memoImagesDirSize は memo-images/ 直下の合計バイト数。フォルダが無ければ 0。
func memoImagesDirSize(dir string) int64 {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return 0
	}
	var total int64
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		if info, err := e.Info(); err == nil {
			total += info.Size()
		}
	}
	return total
}

// handleMemoImages は POST /api/memo-images（リクエスト本文が画像そのもの）と
// GET /api/memo-images/<name> を捌く。POST は保存した画像名を返すだけで、メモへの
// 紐付けは POST /api/memos の images で行う。
func (s *Server) handleMemoImages(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) || !s.memoStoreReady(w) {
		return
	}
	name := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/memo-images"), "/")
	switch r.Method {
	case http.MethodGet:
		if !memoImageNameRe.MatchString(name) {
			writeJSONError(w, http.StatusNotFound, "not_found", "image not found")
			return
		}
		data, err := os.ReadFile(filepath.Join(s.memos.imagesDir, name))
		if err != nil {
			writeJSONError(w, http.StatusNotFound, "not_found", "image not found")
			return
		}
		w.Header().Set("Content-Type", memoImageMime(name))
		w.Header().Set("X-Content-Type-Options", "nosniff")
		// 名前はランダムで中身は書き換えないので、長めにキャッシュしてよい。
		w.Header().Set("Cache-Control", "private, max-age=86400")
		_, _ = w.Write(data)
	case http.MethodPost:
		if name != "" {
			writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "unsupported memo image operation")
			return
		}
		s.handleMemoImageUpload(w, r)
	}
}

func (s *Server) handleMemoImageUpload(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, memoImageMaxBytes)
	data, err := io.ReadAll(r.Body)
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			writeJSONError(w, http.StatusRequestEntityTooLarge, "image_too_large", "image must be 10 MB or smaller")
			return
		}
		writeJSONError(w, http.StatusBadRequest, "bad_request", "could not read image")
		return
	}
	ext, ok := memoImageExtByMime[http.DetectContentType(data)]
	if !ok {
		writeJSONError(w, http.StatusUnsupportedMediaType, "unsupported_image", "image/png, image/jpeg, image/gif, or image/webp required")
		return
	}
	id, err := memoID()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "memo_operation_failed", "memo image could not be saved")
		return
	}
	imageName := id + "." + ext
	dir := s.memos.imagesDir
	if memoImagesDirSize(dir)+int64(len(data)) > memoImagesTotalMaxBytes {
		writeJSONError(w, http.StatusInsufficientStorage, "memo_images_full", "memo images exceed 500 MB in total; delete memos you no longer need")
		return
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "memo_operation_failed", "memo image could not be saved")
		return
	}
	if err := os.WriteFile(filepath.Join(dir, imageName), data, 0o600); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "memo_operation_failed", "memo image could not be saved")
		return
	}
	writeJSON(w, map[string]string{"image": imageName})
}

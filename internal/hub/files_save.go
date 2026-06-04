package hub

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

// filesSaveBodyMaxBytes は POST /api/files-save のリクエスト body 上限（JSON オーバーヘッド込みで 2 MiB）。
// content 自体は filesContentMaxSize（1 MiB）で別途チェックする。
const filesSaveBodyMaxBytes = 2 * 1024 * 1024

// filesSaveReq は POST /api/files-save のリクエスト body。
//   - Path:     書き込み対象ファイルの絶対パス
//   - Content:  保存するテキスト内容
//   - BaseMtime: 競合検出用の基準 mtime（RFC3339。省略可。指定時はサーバ側 mtime と照合する）
type filesSaveReq struct {
	Path      string    `json:"path"`
	Content   string    `json:"content"`
	BaseMtime time.Time `json:"baseMtime"`
}

// filesSaveResp は POST /api/files-save のレスポンス body。
type filesSaveResp struct {
	OK     bool      `json:"ok"`
	Error  string    `json:"error,omitempty"`
	Detail string    `json:"detail,omitempty"`
	Path   string    `json:"path,omitempty"`
	Size   int64     `json:"size,omitempty"`
	Mtime  time.Time `json:"mtime,omitempty"`
}

// handleFilesSave は POST /api/files-save を処理する。
// 検証フロー:
//  1. token + method（POST）
//  2. path が絶対パス必須 + allowed roots スコープ検証
//  3. isTextFile() で許可リスト判定
//  4. content のサイズ上限 filesContentMaxSize（1 MiB）
//  5. 対象ファイルが存在し、ディレクトリでないこと（存在しないファイルへの新規作成は不可）
//  6. baseMtime が指定され、現在の mtime と不一致なら 409 conflict
func (s *Server) handleFilesSave(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	// body を上限付きで読み込む（filesSaveBodyMaxBytes を超えた場合は 413）
	bodyBytes, err := io.ReadAll(io.LimitReader(r.Body, filesSaveBodyMaxBytes+1))
	if err != nil {
		writeSaveErr(w, http.StatusInternalServerError, "read_failed", "cannot read request body", time.Time{})
		return
	}
	if len(bodyBytes) > filesSaveBodyMaxBytes {
		writeSaveErr(w, http.StatusRequestEntityTooLarge, "too_large", "request body exceeds limit", time.Time{})
		return
	}

	var req filesSaveReq
	if err := json.Unmarshal(bodyBytes, &req); err != nil {
		writeSaveErr(w, http.StatusBadRequest, "bad_request", "invalid json", time.Time{})
		return
	}

	// パスの基本検証
	if req.Path == "" {
		writeSaveErr(w, http.StatusBadRequest, "bad_request", "path is required", time.Time{})
		return
	}
	if !filepath.IsAbs(req.Path) {
		writeSaveErr(w, http.StatusBadRequest, "bad_request", "path must be an absolute path", time.Time{})
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	pathClean := filepath.Clean(req.Path)
	if ok, _ := isPathUnderAllowedRoots(pathClean, cwd, gitRoot); !ok {
		writeSaveErr(w, http.StatusForbidden, "forbidden", "path is outside allowed roots", time.Time{})
		return
	}

	// テキストファイル許可リスト判定（読み取りと同一リスト）
	if !isTextFile(pathClean) {
		writeSaveErr(w, http.StatusForbidden, "forbidden", "not a previewable text file", time.Time{})
		return
	}

	// サイズ上限チェック（1 MiB）
	if len(req.Content) > filesContentMaxSize {
		writeSaveErr(w, http.StatusRequestEntityTooLarge, "too_large", "content exceeds 1 MiB limit", time.Time{})
		return
	}

	// ファイル存在確認
	info, err := os.Stat(pathClean)
	if err != nil {
		if os.IsNotExist(err) {
			writeSaveErr(w, http.StatusNotFound, "not_found", "file not found (new file creation is not supported)", time.Time{})
			return
		}
		writeSaveErr(w, http.StatusInternalServerError, "internal_error", errorDetail("stat failed", err), time.Time{})
		return
	}
	if info.IsDir() {
		writeSaveErr(w, http.StatusBadRequest, "bad_request", "path is a directory", time.Time{})
		return
	}

	// baseMtime が指定されている場合、競合検出
	if !req.BaseMtime.IsZero() {
		currentMtime := info.ModTime().UTC().Truncate(time.Second)
		baseMtime := req.BaseMtime.UTC().Truncate(time.Second)
		if !currentMtime.Equal(baseMtime) {
			writeSaveErr(w, http.StatusConflict, "conflict", "file was modified by another process", currentMtime)
			return
		}
	}

	// 既存ファイルのパーミッションを引き継いで書き込み
	perm := info.Mode().Perm()
	if err := os.WriteFile(pathClean, []byte(req.Content), perm); err != nil {
		writeSaveErr(w, http.StatusInternalServerError, "write_failed", errorDetail("write failed", err), time.Time{})
		return
	}

	// 書き込み後の mtime を取得してレスポンスに含める
	newInfo, err := os.Stat(pathClean)
	if err != nil {
		writeSaveErr(w, http.StatusInternalServerError, "stat_after_write_failed", errorDetail("stat after write failed", err), time.Time{})
		return
	}

	writeJSON(w, filesSaveResp{
		OK:    true,
		Path:  pathClean,
		Size:  newInfo.Size(),
		Mtime: newInfo.ModTime(),
	})
}

// writeSaveErr は files-save 系のエラーレスポンスを書き込む。
// currentMtime が非ゼロの場合は 409 競合レスポンスに現在の mtime を含める。
func writeSaveErr(w http.ResponseWriter, status int, code, detail string, currentMtime time.Time) {
	resp := filesSaveResp{OK: false, Error: code, Detail: detail}
	if !currentMtime.IsZero() {
		resp.Mtime = currentMtime
	}
	writeJSONStatus(w, status, resp)
}

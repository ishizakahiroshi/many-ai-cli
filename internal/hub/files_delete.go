package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// filesDeleteDirReq は POST /api/files-delete-dir のリクエスト body。
// Src は削除対象ディレクトリの絶対パス。
type filesDeleteDirReq struct {
	Src string `json:"src"`
}

type filesDeleteDirResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error,omitempty"`
	Detail string `json:"detail,omitempty"`
}

// handleFilesDeleteDir は POST /api/files-delete-dir を処理する。
// ディレクトリだけを削除対象にし、ファイル・シンボリックリンク・許可ルート自身は拒否する。
func (s *Server) handleFilesDeleteDir(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	var req filesDeleteDirReq
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.Src == "" {
		writeDeleteDirErr(w, http.StatusBadRequest, "bad_request", "src is required")
		return
	}
	if !filepath.IsAbs(req.Src) {
		writeDeleteDirErr(w, http.StatusBadRequest, "bad_request", "src must be an absolute path")
		return
	}

	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	srcClean := filepath.Clean(req.Src)
	if ok, _ := isPathUnderAllowedRoots(srcClean, cwd, gitRoot); !ok {
		writeDeleteDirErr(w, http.StatusForbidden, "forbidden", "src is outside allowed roots")
		return
	}
	if pathsEqual(srcClean, cwd) || pathsEqual(srcClean, gitRoot) {
		writeDeleteDirErr(w, http.StatusConflict, "conflict", "refusing to delete an allowed root directory")
		return
	}

	info, err := os.Lstat(srcClean)
	if err != nil {
		writeDeleteDirErr(w, http.StatusNotFound, "not_found", errorDetail("src not found", err))
		return
	}
	if !info.IsDir() {
		writeDeleteDirErr(w, http.StatusBadRequest, "bad_request", "src must be a directory")
		return
	}

	entries, err := os.ReadDir(srcClean)
	if err != nil {
		writeDeleteDirErr(w, http.StatusInternalServerError, "read_failed", errorDetail("read directory failed", err))
		return
	}
	if len(entries) > 0 {
		writeDeleteDirErr(w, http.StatusConflict, "conflict", "src directory must be empty")
		return
	}

	if err := os.Remove(srcClean); err != nil {
		if isDirectoryNotEmptyError(err) {
			writeDeleteDirErr(w, http.StatusConflict, "conflict", "src directory must be empty")
			return
		}
		writeDeleteDirErr(w, http.StatusInternalServerError, "delete_failed", errorDetail("delete failed", err))
		return
	}

	writeJSON(w, filesDeleteDirResp{OK: true})
}

func writeDeleteDirErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesDeleteDirResp{OK: false, Error: code, Detail: detail})
}

func isDirectoryNotEmptyError(err error) bool {
	if err == nil {
		return false
	}
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "not empty") || strings.Contains(msg, "directory not empty")
}

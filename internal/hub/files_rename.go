package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// filesRenameReq は POST /api/files-rename のリクエスト body。
//   - Src:     リネーム対象ディレクトリの絶対パス
//   - NewName: 新しい basename（ディレクトリセパレータを含まないこと）
type filesRenameReq struct {
	Src     string `json:"src"`
	NewName string `json:"newName"`
}

type filesRenameResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error,omitempty"`
	Detail string `json:"detail,omitempty"`
	NewAbs string `json:"newAbs,omitempty"`
}

// handleFilesRename は POST /api/files-rename を処理する。
// src と同じディレクトリ内で basename だけを newName に変更する。
// 検証:
//  1. token
//  2. method (POST)
//  3. src が絶対パス
//  4. newName が basename のみ（空でない、"." ".." でない、セパレータを含まない）
//  5. src 存在、かつディレクトリ
//  6. src が allowed roots（cwd または gitRoot）配下
//  7. newName が src の現在の basename と異なる（no-op 拒否）
//  8. 移動先（同ディレクトリ/newName）が既存でない（上書き禁止）
func (s *Server) handleFilesRename(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	var req filesRenameReq
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.Src == "" || req.NewName == "" {
		writeRenameErr(w, http.StatusBadRequest, "bad_request", "src and newName are required")
		return
	}
	if !filepath.IsAbs(req.Src) {
		writeRenameErr(w, http.StatusBadRequest, "bad_request", "src must be an absolute path")
		return
	}
	if !isSafeBasename(req.NewName) {
		writeRenameErr(w, http.StatusBadRequest, "bad_request", "newName must be a plain file name without path separators")
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	srcClean := filepath.Clean(req.Src)
	if ok, _ := isPathUnderAllowedRoots(srcClean, cwd, gitRoot); !ok {
		writeRenameErr(w, http.StatusForbidden, "forbidden", "src is outside allowed roots")
		return
	}

	info, err := os.Lstat(srcClean)
	if err != nil {
		writeRenameErr(w, http.StatusNotFound, "not_found", errorDetail("src not found", err))
		return
	}
	if !info.IsDir() {
		writeRenameErr(w, http.StatusBadRequest, "bad_request", "src must be a directory")
		return
	}

	if req.NewName == filepath.Base(srcClean) {
		writeRenameErr(w, http.StatusConflict, "conflict", "newName is identical to current name")
		return
	}

	newPath := filepath.Join(filepath.Dir(srcClean), req.NewName)
	if _, err := os.Lstat(newPath); err == nil {
		writeRenameErr(w, http.StatusConflict, "conflict", "target already exists: "+newPath)
		return
	}

	if err := os.Rename(srcClean, newPath); err != nil {
		writeRenameErr(w, http.StatusInternalServerError, "rename_failed", errorDetail("rename failed", err))
		return
	}

	writeJSON(w, filesRenameResp{OK: true, NewAbs: newPath})
}

func writeRenameErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesRenameResp{OK: false, Error: code, Detail: detail})
}

// isSafeBasename は name が path separator や ".." を含まない単純な basename か検証する。
func isSafeBasename(name string) bool {
	if name == "" || name == "." || name == ".." {
		return false
	}
	if strings.ContainsAny(name, `/\`) {
		return false
	}
	// Windows のドライブレター指定（"C:foo" 等）も拒否
	if strings.Contains(name, ":") {
		return false
	}
	return true
}

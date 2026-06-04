package hub

import (
	"errors"
	"net/http"
	"os"
	"path/filepath"

	"any-ai-cli/internal/config"
)

// filesMkdirReq は POST /api/files-mkdir のリクエスト body。
//   - Dir:  新規ディレクトリを作成する親ディレクトリの絶対パス
//   - Name: 作成するディレクトリの basename（ディレクトリセパレータを含まないこと）
type filesMkdirReq struct {
	Dir  string `json:"dir"`
	Name string `json:"name"`
}

type filesMkdirResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error,omitempty"`
	Detail string `json:"detail,omitempty"`
	NewAbs string `json:"newAbs,omitempty"`
}

// handleFilesMkdir は POST /api/files-mkdir を処理する。
// 指定した親ディレクトリ（dir）の直下に name という名前のディレクトリを 1 段だけ作成する。
// 親ディレクトリが存在しない場合はエラーとし、os.MkdirAll は使わない（誤操作防止）。
// 検証:
//  1. token
//  2. method (POST)
//  3. dir が絶対パス
//  4. name が basename のみ（空でない、"." ".." でない、セパレータを含まない）
//  5. dir が allowed roots（cwd または gitRoot）配下
//  6. dir が実在する（os.Stat）
//  7. 作成先（dir/name）が既存でない（上書き禁止）
func (s *Server) handleFilesMkdir(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	var req filesMkdirReq
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.Dir == "" || req.Name == "" {
		writeMkdirErr(w, http.StatusBadRequest, "bad_request", "dir and name are required")
		return
	}
	if !filepath.IsAbs(req.Dir) {
		writeMkdirErr(w, http.StatusBadRequest, "bad_request", "dir must be an absolute path")
		return
	}
	if !isSafeBasename(req.Name) {
		writeMkdirErr(w, http.StatusBadRequest, "bad_request", "name must be a plain directory name without path separators")
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	dirClean := filepath.Clean(req.Dir)
	if ok, _ := isPathUnderAllowedRoots(dirClean, cwd, gitRoot); !ok {
		writeMkdirErr(w, http.StatusForbidden, "forbidden", "dir is outside allowed roots")
		return
	}

	info, err := os.Stat(dirClean)
	if err != nil {
		writeMkdirErr(w, http.StatusBadRequest, "bad_request", errorDetail("parent directory not found", err))
		return
	}
	if !info.IsDir() {
		writeMkdirErr(w, http.StatusBadRequest, "bad_request", "dir must be a directory")
		return
	}

	newPath := filepath.Join(dirClean, req.Name)
	if _, err := os.Lstat(newPath); err == nil {
		writeMkdirErr(w, http.StatusConflict, "already_exists", "target already exists: "+newPath)
		return
	}

	if err := os.Mkdir(newPath, config.DirMode); err != nil {
		if errors.Is(err, os.ErrExist) {
			writeMkdirErr(w, http.StatusConflict, "already_exists", "target already exists: "+newPath)
			return
		}
		if errors.Is(err, os.ErrNotExist) {
			writeMkdirErr(w, http.StatusBadRequest, "bad_request", errorDetail("parent directory not found", err))
			return
		}
		writeMkdirErr(w, http.StatusInternalServerError, "mkdir_failed", errorDetail("mkdir failed", err))
		return
	}

	writeJSON(w, filesMkdirResp{OK: true, NewAbs: newPath})
}

func writeMkdirErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesMkdirResp{OK: false, Error: code, Detail: detail})
}

package hub

import (
	"errors"
	"net/http"
	"os"
	"path/filepath"
)

// filesCreateReq は POST /api/files-create のリクエスト body。
//   - Dir:  新規ファイルを作成する親ディレクトリの絶対パス
//   - Name: 作成するファイルの basename（ディレクトリセパレータを含まないこと）
type filesCreateReq struct {
	Dir  string `json:"dir"`
	Name string `json:"name"`
}

type filesCreateResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error,omitempty"`
	Detail string `json:"detail,omitempty"`
	NewAbs string `json:"newAbs,omitempty"`
}

// handleFilesCreate は POST /api/files-create を処理する。
// 指定した親ディレクトリ（dir）の直下に name という名前の空ファイルを 1 つ作成する。
// 既存ファイルの上書きや中間ディレクトリの自動作成はしない。
func (s *Server) handleFilesCreate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	var req filesCreateReq
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.Dir == "" || req.Name == "" {
		writeCreateErr(w, http.StatusBadRequest, "bad_request", "dir and name are required")
		return
	}
	if !filepath.IsAbs(req.Dir) {
		writeCreateErr(w, http.StatusBadRequest, "bad_request", "dir must be an absolute path")
		return
	}
	if !isSafeBasename(req.Name) {
		writeCreateErr(w, http.StatusBadRequest, "bad_request", "name must be a plain file name without path separators")
		return
	}

	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	dirClean := filepath.Clean(req.Dir)
	if ok, _ := isPathUnderAllowedRoots(dirClean, cwd, gitRoot); !ok {
		writeCreateErr(w, http.StatusForbidden, "forbidden", "dir is outside allowed roots")
		return
	}

	info, err := os.Stat(dirClean)
	if err != nil {
		writeCreateErr(w, http.StatusBadRequest, "bad_request", errorDetail("parent directory not found", err))
		return
	}
	if !info.IsDir() {
		writeCreateErr(w, http.StatusBadRequest, "bad_request", "dir must be a directory")
		return
	}

	newPath := filepath.Join(dirClean, req.Name)
	if ok, _ := isPathUnderAllowedRoots(newPath, cwd, gitRoot); !ok {
		writeCreateErr(w, http.StatusForbidden, "forbidden", "target is outside allowed roots")
		return
	}
	if !isTextFile(newPath) {
		writeCreateErr(w, http.StatusForbidden, "not_previewable", "not a previewable text file")
		return
	}
	if _, err := os.Lstat(newPath); err == nil {
		writeCreateErr(w, http.StatusConflict, "already_exists", "target already exists: "+newPath)
		return
	}

	f, err := os.OpenFile(newPath, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
	if err != nil {
		if errors.Is(err, os.ErrExist) {
			writeCreateErr(w, http.StatusConflict, "already_exists", "target already exists: "+newPath)
			return
		}
		if errors.Is(err, os.ErrNotExist) {
			writeCreateErr(w, http.StatusBadRequest, "bad_request", errorDetail("parent directory not found", err))
			return
		}
		writeCreateErr(w, http.StatusInternalServerError, "create_failed", errorDetail("create failed", err))
		return
	}
	if err := f.Close(); err != nil {
		writeCreateErr(w, http.StatusInternalServerError, "create_failed", errorDetail("close failed", err))
		return
	}

	writeJSON(w, filesCreateResp{OK: true, NewAbs: newPath})
}

func writeCreateErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesCreateResp{OK: false, Error: code, Detail: detail})
}

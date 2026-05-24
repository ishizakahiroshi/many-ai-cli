package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
)

// filesDeleteDirReq は POST /api/files-delete-dir のリクエスト body。
// Src は削除対象ディレクトリの絶対パス。
type filesDeleteDirReq struct {
	Src string `json:"src"`
}

type filesDeleteDirResp struct {
	OK    bool   `json:"ok"`
	Error string `json:"error,omitempty"`
}

// handleFilesDeleteDir は POST /api/files-delete-dir を処理する。
// ディレクトリだけを削除対象にし、ファイル・シンボリックリンク・許可ルート自身は拒否する。
func (s *Server) handleFilesDeleteDir(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	w.Header().Set("Content-Type", "application/json")

	var req filesDeleteDirReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeDeleteDirErr(w, "bad request: "+err.Error())
		return
	}
	if req.Src == "" {
		writeDeleteDirErr(w, "src is required")
		return
	}
	if !filepath.IsAbs(req.Src) {
		writeDeleteDirErr(w, "src must be an absolute path")
		return
	}

	cwd := s.hubCWD
	if sidStr := r.URL.Query().Get("session"); sidStr != "" {
		if sid, err := strconv.Atoi(sidStr); err == nil {
			s.mu.Lock()
			if ses := s.sessions[sid]; ses != nil {
				cwd = ses.CWD
			}
			s.mu.Unlock()
		}
	}
	gitRoot := findGitRoot(cwd)

	srcClean := filepath.Clean(req.Src)
	if ok, _ := isPathUnderAllowedRoots(srcClean, cwd, gitRoot); !ok {
		writeDeleteDirErr(w, "forbidden: src is outside allowed roots")
		return
	}
	if pathsEqual(srcClean, cwd) || pathsEqual(srcClean, gitRoot) {
		writeDeleteDirErr(w, "refusing to delete an allowed root directory")
		return
	}

	info, err := os.Lstat(srcClean)
	if err != nil {
		writeDeleteDirErr(w, "src not found: "+err.Error())
		return
	}
	if !info.IsDir() {
		writeDeleteDirErr(w, "src must be a directory")
		return
	}

	if err := os.RemoveAll(srcClean); err != nil {
		writeDeleteDirErr(w, "delete failed: "+err.Error())
		return
	}

	_ = json.NewEncoder(w).Encode(filesDeleteDirResp{OK: true})
}

func writeDeleteDirErr(w http.ResponseWriter, msg string) {
	_ = json.NewEncoder(w).Encode(filesDeleteDirResp{OK: false, Error: msg})
}

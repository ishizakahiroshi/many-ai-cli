package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
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
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	w.Header().Set("Content-Type", "application/json")

	var req filesRenameReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeRenameErr(w, "bad request: "+err.Error())
		return
	}
	if req.Src == "" || req.NewName == "" {
		writeRenameErr(w, "src and newName are required")
		return
	}
	if !filepath.IsAbs(req.Src) {
		writeRenameErr(w, "src must be an absolute path")
		return
	}
	if !isSafeBasename(req.NewName) {
		writeRenameErr(w, "newName must be a plain file name without path separators")
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
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
		writeRenameErr(w, "forbidden: src is outside allowed roots")
		return
	}

	info, err := os.Lstat(srcClean)
	if err != nil {
		writeRenameErr(w, "src not found: "+err.Error())
		return
	}
	if !info.IsDir() {
		writeRenameErr(w, "src must be a directory")
		return
	}

	if req.NewName == filepath.Base(srcClean) {
		writeRenameErr(w, "newName is identical to current name")
		return
	}

	newPath := filepath.Join(filepath.Dir(srcClean), req.NewName)
	if _, err := os.Lstat(newPath); err == nil {
		writeRenameErr(w, "target already exists: "+newPath)
		return
	}

	if err := os.Rename(srcClean, newPath); err != nil {
		writeRenameErr(w, "rename failed: "+err.Error())
		return
	}

	_ = json.NewEncoder(w).Encode(filesRenameResp{OK: true, NewAbs: newPath})
}

func writeRenameErr(w http.ResponseWriter, msg string) {
	_ = json.NewEncoder(w).Encode(filesRenameResp{OK: false, Error: msg})
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

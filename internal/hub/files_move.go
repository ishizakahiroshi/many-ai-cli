package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// filesMoveReq は POST /api/files-move のリクエスト body。
//   - Src: 移動対象（ファイル or ディレクトリ）の絶対パス
//   - DstDir: 移動先ディレクトリの絶対パス（最終パスは filepath.Join(DstDir, filepath.Base(Src))）
type filesMoveReq struct {
	Src    string `json:"src"`
	DstDir string `json:"dstDir"`
}

type filesMoveResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error,omitempty"`
	NewAbs string `json:"newAbs,omitempty"`
}

// handleFilesMove は POST /api/files-move を処理する。
// src を dstDir 配下へ os.Rename で移動する。
// 検証:
//  1. token
//  2. method (POST)
//  3. 両パスが絶対
//  4. src 存在、dstDir 存在＆ディレクトリ
//  5. 両パスが allowed roots（cwd または gitRoot）配下
//  6. src と dstDir が同一ディレクトリでない（no-op 拒否）
//  7. dstDir が src 自身または src の配下でない（自己への移動禁止）
//  8. 移動先（dstDir/basename(src)）が既存でない（上書き禁止）
func (s *Server) handleFilesMove(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	w.Header().Set("Content-Type", "application/json")

	var req filesMoveReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeMoveErr(w, "bad request: "+err.Error())
		return
	}
	if req.Src == "" || req.DstDir == "" {
		writeMoveErr(w, "src and dstDir are required")
		return
	}
	if !filepath.IsAbs(req.Src) || !filepath.IsAbs(req.DstDir) {
		writeMoveErr(w, "src and dstDir must be absolute paths")
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

	// allowed roots 配下チェック
	if ok, _ := isPathUnderAllowedRoots(req.Src, cwd, gitRoot); !ok {
		writeMoveErr(w, "forbidden: src is outside allowed roots")
		return
	}
	if ok, _ := isPathUnderAllowedRoots(req.DstDir, cwd, gitRoot); !ok {
		writeMoveErr(w, "forbidden: dstDir is outside allowed roots")
		return
	}

	srcClean := filepath.Clean(req.Src)
	dstDirClean := filepath.Clean(req.DstDir)

	srcInfo, err := os.Lstat(srcClean)
	if err != nil {
		writeMoveErr(w, "src not found: "+err.Error())
		return
	}
	dstDirInfo, err := os.Stat(dstDirClean)
	if err != nil {
		writeMoveErr(w, "dstDir not found: "+err.Error())
		return
	}
	if !dstDirInfo.IsDir() {
		writeMoveErr(w, "dstDir is not a directory")
		return
	}

	// 同一ディレクトリへの no-op
	srcParent := filepath.Dir(srcClean)
	if pathsEqual(srcParent, dstDirClean) {
		writeMoveErr(w, "src is already in dstDir")
		return
	}

	// src 自身または src の配下に dstDir を入れようとする操作を禁止
	if srcInfo.IsDir() {
		if pathsEqual(srcClean, dstDirClean) || isUnder(dstDirClean, srcClean) {
			writeMoveErr(w, "cannot move a directory into itself or its descendant")
			return
		}
	}

	newPath := filepath.Join(dstDirClean, filepath.Base(srcClean))
	if _, err := os.Lstat(newPath); err == nil {
		writeMoveErr(w, "target already exists: "+newPath)
		return
	}

	if err := os.Rename(srcClean, newPath); err != nil {
		writeMoveErr(w, "rename failed: "+err.Error())
		return
	}

	_ = json.NewEncoder(w).Encode(filesMoveResp{OK: true, NewAbs: newPath})
}

func writeMoveErr(w http.ResponseWriter, msg string) {
	_ = json.NewEncoder(w).Encode(filesMoveResp{OK: false, Error: msg})
}

// pathsEqual は 2 つのパスを Clean したうえで比較する。
// Windows のドライブレターは大文字小文字を無視する。
func pathsEqual(a, b string) bool {
	ca := filepath.Clean(a)
	cb := filepath.Clean(b)
	if ca == cb {
		return true
	}
	// Windows のドライブレター違い対策（例: C:\foo vs c:\foo）
	return strings.EqualFold(ca, cb)
}

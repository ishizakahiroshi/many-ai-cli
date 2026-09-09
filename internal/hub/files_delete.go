package hub

import (
	"net/http"
	"os"
	"path/filepath"
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
	for _, root := range []string{cwd, gitRoot} {
		if root == "" {
			continue
		}
		same, known := protectedPathIdentityEqual(srcClean, root)
		if !known {
			writeDeleteDirErr(w, http.StatusConflict, "conflict", "cannot establish allowed root identity")
			return
		}
		if same {
			writeDeleteDirErr(w, http.StatusConflict, "conflict", "refusing to delete an allowed root directory")
			return
		}
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

	if err := os.RemoveAll(srcClean); err != nil {
		writeDeleteDirErr(w, http.StatusInternalServerError, "delete_failed", errorDetail("delete failed", err))
		return
	}

	writeJSON(w, filesDeleteDirResp{OK: true})
}

// protectedPathIdentityEqual is intentionally local to delete-root
// protection. General path comparisons keep their lexical semantics for
// rename and list operations, while this check must also recognize a CWD
// reached through a symlink or junction.
func protectedPathIdentityEqual(target, root string) (same, known bool) {
	resolvedTarget, targetOK := evalSymlinksViaSelf(filepath.Clean(target))
	resolvedRoot, rootOK := evalSymlinksViaSelf(filepath.Clean(root))
	if targetOK && rootOK {
		if pathsEqual(resolvedTarget, resolvedRoot) {
			return true, true
		}
		return false, true
	}
	targetInfo, targetErr := os.Stat(target)
	rootInfo, rootErr := os.Stat(root)
	if targetErr == nil && rootErr == nil {
		return os.SameFile(targetInfo, rootInfo), true
	}
	return false, false
}

func writeDeleteDirErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesDeleteDirResp{OK: false, Error: code, Detail: detail})
}

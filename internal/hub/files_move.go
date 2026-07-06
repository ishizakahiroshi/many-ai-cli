package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

// filesMoveReq は POST /api/files-move のリクエスト body。
//   - Src/DstDir: 単ファイルモード（後方互換）
//   - Srcs/DstDir: 多ファイルモード（Srcs が空でない場合に優先）
type filesMoveReq struct {
	Src    string   `json:"src"`
	DstDir string   `json:"dstDir"`
	Srcs   []string `json:"srcs"`
}

type fileMoveResult struct {
	Src    string `json:"src"`
	NewAbs string `json:"newAbs,omitempty"`
	Error  string `json:"error,omitempty"`
}

type filesMoveResp struct {
	OK      bool             `json:"ok"`
	Error   string           `json:"error,omitempty"`
	Detail  string           `json:"detail,omitempty"`
	NewAbs  string           `json:"newAbs,omitempty"`  // 単ファイル後方互換
	Results []fileMoveResult `json:"results,omitempty"` // 多ファイル時
}

type fileMovePlan struct {
	SrcClean string
	NewPath  string
	SrcInfo  os.FileInfo
}

// processSingleMove は src → dstDir への移動を実行する。
// dstDir は呼び出し元で検証済みであること（存在・ディレクトリ・allowed roots）。
func (s *Server) processSingleMove(src, dstDir, cwd, gitRoot string) fileMoveResult {
	plan, res := s.planSingleMove(src, filepath.Clean(dstDir), cwd, gitRoot)
	if res.Error != "" {
		return res
	}
	if err := os.Rename(plan.SrcClean, plan.NewPath); err != nil {
		return fileMoveResult{Src: src, Error: "rename failed: " + err.Error()}
	}
	return fileMoveResult{Src: src, NewAbs: plan.NewPath}
}

func (s *Server) planSingleMove(src, dstDirClean, cwd, gitRoot string) (fileMovePlan, fileMoveResult) {
	if src == "" {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "src is required"}
	}
	if !filepath.IsAbs(src) {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "src must be an absolute path"}
	}
	if ok, _ := isPathUnderAllowedRoots(src, cwd, gitRoot); !ok {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "forbidden: src is outside allowed roots"}
	}
	srcClean := filepath.Clean(src)

	srcInfo, err := os.Lstat(srcClean)
	if err != nil {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "src not found: " + err.Error()}
	}
	srcParent := filepath.Dir(srcClean)
	if pathsEqual(srcParent, dstDirClean) {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "src is already in dstDir"}
	}
	if srcInfo.IsDir() {
		if pathsEqual(srcClean, dstDirClean) || isUnder(dstDirClean, srcClean) {
			return fileMovePlan{}, fileMoveResult{Src: src, Error: "cannot move a directory into itself or its descendant"}
		}
	}
	newPath := filepath.Join(dstDirClean, filepath.Base(srcClean))
	if _, err := os.Lstat(newPath); err == nil {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "target already exists: " + newPath}
	} else if !os.IsNotExist(err) {
		return fileMovePlan{}, fileMoveResult{Src: src, Error: "target check failed: " + err.Error()}
	}
	return fileMovePlan{SrcClean: srcClean, NewPath: newPath, SrcInfo: srcInfo}, fileMoveResult{Src: src}
}

func (s *Server) processMultiMove(srcs []string, dstDirClean, cwd, gitRoot string) filesMoveResp {
	results := make([]fileMoveResult, len(srcs))
	plans := make([]fileMovePlan, len(srcs))
	allOK := true
	srcSeen := map[string]int{}
	targetSeen := map[string]int{}

	for i, src := range srcs {
		results[i].Src = src
		plan, res := s.planSingleMove(src, dstDirClean, cwd, gitRoot)
		plans[i] = plan
		if res.Error != "" {
			results[i].Error = res.Error
			allOK = false
			continue
		}
		srcKey := movePathKey(plan.SrcClean)
		if prev, ok := srcSeen[srcKey]; ok {
			results[i].Error = "duplicate source path"
			if results[prev].Error == "" {
				results[prev].Error = "duplicate source path"
			}
			allOK = false
		} else {
			srcSeen[srcKey] = i
		}
		targetKey := movePathKey(plan.NewPath)
		if prev, ok := targetSeen[targetKey]; ok {
			results[i].Error = "multiple sources would overwrite the same target: " + plan.NewPath
			if results[prev].Error == "" {
				results[prev].Error = "multiple sources would overwrite the same target: " + plan.NewPath
			}
			allOK = false
		} else {
			targetSeen[targetKey] = i
		}
	}

	for i := range plans {
		if plans[i].SrcClean == "" || !plans[i].SrcInfo.IsDir() {
			continue
		}
		for j := range plans {
			if i == j || plans[j].SrcClean == "" {
				continue
			}
			if !pathsEqual(plans[i].SrcClean, plans[j].SrcClean) && isUnder(plans[j].SrcClean, plans[i].SrcClean) {
				if results[i].Error == "" {
					results[i].Error = "cannot move a directory together with one of its descendants"
				}
				if results[j].Error == "" {
					results[j].Error = "cannot move a path together with its ancestor directory"
				}
				allOK = false
			}
		}
	}

	if !allOK {
		return filesMoveResp{OK: false, Error: "move_preflight_failed", Detail: "move preflight failed", Results: results}
	}

	completed := make([]fileMovePlan, 0, len(plans))
	for i, plan := range plans {
		if err := os.Rename(plan.SrcClean, plan.NewPath); err != nil {
			results[i].Error = "rename failed: " + err.Error()
			rollbackErrs := rollbackMoves(completed)
			errMsg := results[i].Error
			if len(rollbackErrs) > 0 {
				errMsg += "; rollback failed: " + strings.Join(rollbackErrs, "; ")
			}
			return filesMoveResp{OK: false, Error: "rename_failed", Detail: errMsg, Results: results}
		}
		completed = append(completed, plan)
	}
	for i, plan := range plans {
		results[i].NewAbs = plan.NewPath
	}

	return filesMoveResp{OK: true, Results: results}
}

func rollbackMoves(plans []fileMovePlan) []string {
	errs := []string{}
	for i := len(plans) - 1; i >= 0; i-- {
		plan := plans[i]
		if err := os.Rename(plan.NewPath, plan.SrcClean); err != nil {
			errs = append(errs, plan.NewPath+" -> "+plan.SrcClean+": "+err.Error())
		}
	}
	return errs
}

func movePathKey(path string) string {
	cleaned := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		return strings.ToLower(cleaned)
	}
	return cleaned
}

// handleFilesMove は POST /api/files-move を処理する。
// 単ファイルモード（src/dstDir）と多ファイルモード（srcs/dstDir）の両方をサポートする。
func (s *Server) handleFilesMove(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}

	var req filesMoveReq
	if !decodeJSON(w, r, &req) {
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)

	// dstDir の共通バリデーション
	if req.DstDir == "" {
		writeMoveErr(w, http.StatusBadRequest, "bad_request", "dstDir is required")
		return
	}
	if !filepath.IsAbs(req.DstDir) {
		writeMoveErr(w, http.StatusBadRequest, "bad_request", "dstDir must be an absolute path")
		return
	}
	if ok, _ := isPathUnderAllowedRoots(req.DstDir, cwd, gitRoot); !ok {
		writeMoveErr(w, http.StatusForbidden, "forbidden", "dstDir is outside allowed roots")
		return
	}
	dstDirClean := filepath.Clean(req.DstDir)
	dstDirInfo, err := os.Stat(dstDirClean)
	if err != nil {
		writeMoveErr(w, http.StatusNotFound, "not_found", errorDetail("dstDir not found", err))
		return
	}
	if !dstDirInfo.IsDir() {
		writeMoveErr(w, http.StatusBadRequest, "bad_request", "dstDir is not a directory")
		return
	}

	// 多ファイルモード (Srcs)
	if len(req.Srcs) > 0 {
		resp := s.processMultiMove(req.Srcs, dstDirClean, cwd, gitRoot)
		if !resp.OK {
			writeJSONStatus(w, http.StatusBadRequest, resp)
			return
		}
		writeJSON(w, resp)
		return
	}

	// 単ファイルモード（後方互換）
	if req.Src == "" {
		writeMoveErr(w, http.StatusBadRequest, "bad_request", "src or srcs is required")
		return
	}
	res := s.processSingleMove(req.Src, req.DstDir, cwd, gitRoot)
	if res.Error != "" {
		status, code := fileOperationErrorStatus(res.Error)
		writeMoveErr(w, status, code, res.Error)
		return
	}
	writeJSON(w, filesMoveResp{OK: true, NewAbs: res.NewAbs})
}

func writeMoveErr(w http.ResponseWriter, status int, code, detail string) {
	writeJSONStatus(w, status, filesMoveResp{OK: false, Error: code, Detail: detail})
}

func fileOperationErrorStatus(msg string) (int, string) {
	low := strings.ToLower(msg)
	switch {
	case strings.Contains(low, "forbidden"):
		return http.StatusForbidden, "forbidden"
	case strings.Contains(low, "not found"):
		return http.StatusNotFound, "not_found"
	case strings.Contains(low, "already exists"),
		strings.Contains(low, "already in"),
		strings.Contains(low, "duplicate"),
		strings.Contains(low, "overwrite"),
		strings.Contains(low, "descendant"):
		return http.StatusConflict, "conflict"
	case strings.Contains(low, "failed"):
		return http.StatusInternalServerError, "operation_failed"
	default:
		return http.StatusBadRequest, "bad_request"
	}
}

// pathsEqual は 2 つのパスを Clean したうえで比較する。
// Windows のドライブレターは大文字小文字を無視するが、case-sensitive な FS
// (Linux/macOS) では `Foo` と `foo` は別物として扱う（リネーム・削除の誤判定を防ぐ）。
func pathsEqual(a, b string) bool {
	ca := filepath.Clean(a)
	cb := filepath.Clean(b)
	if ca == cb {
		return true
	}
	// Windows パス（C:\foo 等）またはランタイムが Windows のときのみ大小文字を無視する。
	// files_list.go の isUnder と同じガードを使う。
	if runtime.GOOS == "windows" || isWindowsPath(ca) || isWindowsPath(cb) {
		return strings.EqualFold(ca, cb)
	}
	return false
}

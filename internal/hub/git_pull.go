package hub

import (
	"context"
	"net/http"
	"strings"
	"time"
)

const gitPullTimeout = 30 * time.Second

type gitPullReq struct {
	Session int    `json:"session"`
	Token   string `json:"token"`
}

type gitPullResp struct {
	OK     bool   `json:"ok"`
	Output string `json:"output,omitempty"`
}

// handleGitPull は POST /api/git-pull を処理する。
// リモート追跡ブランチの内容を fast-forward でローカルブランチへ追従させる
// （git pull --ff-only）。分岐していて ff 不可の場合はマージコミットを作らず失敗を返す。
func (s *Server) handleGitPull(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req gitPullReq
	if !decodeJSON(w, r, &req) {
		return
	}
	sid := req.Session
	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), gitPullTimeout)
	defer cancel()
	out, runErr := runGitCombined(ctx, cwd, "pull", "--ff-only")
	if runErr != nil {
		code, status := classifyGitPullError(string(out))
		s.logger.Warn("git pull failed", "session_id", sid, "err", runErr, "output", string(out))
		writeGitError(w, status, code, sanitizeGitErrMsg(runErr))
		return
	}
	writeJSON(w, gitPullResp{OK: true, Output: string(out)})
}

// classifyGitPullError は git pull --ff-only の combined output から
// エラー種別コードと HTTP ステータスを判定する。
//
//   - not_fast_forward: 分岐していて fast-forward 不可（マージコミットを作らず停止）
//   - local_changes:    未コミット変更があり上書きを避けて停止
//   - no_upstream:      upstream（追跡ブランチ）未設定
//   - git_command_failed: 上記以外
func classifyGitPullError(out string) (code string, status int) {
	msg := strings.ToLower(out)
	switch {
	case strings.Contains(msg, "not possible to fast-forward"),
		strings.Contains(msg, "non-fast-forward"):
		return "not_fast_forward", http.StatusConflict
	case strings.Contains(msg, "local changes") && strings.Contains(msg, "overwritten"),
		strings.Contains(msg, "would be overwritten by merge"),
		strings.Contains(msg, "please commit your changes or stash them"):
		return "local_changes", http.StatusConflict
	case strings.Contains(msg, "no tracking information"),
		strings.Contains(msg, "no upstream"):
		return "no_upstream", http.StatusBadRequest
	default:
		return "git_command_failed", http.StatusInternalServerError
	}
}

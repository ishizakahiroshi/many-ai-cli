package hub

import (
	"context"
	"net/http"
	"strings"
	"time"
)

const gitPushTimeout = 60 * time.Second

type gitPushReq struct {
	Session int    `json:"session"`
	Token   string `json:"token"`
}

type gitPushResp struct {
	OK     bool   `json:"ok"`
	Output string `json:"output,omitempty"`
}

func gitPushNonInteractiveEnv() []string {
	return []string{
		"GIT_TERMINAL_PROMPT=0",
		"GIT_ASKPASS=echo",
	}
}

// handleGitPush は POST /api/git-push を処理する。
// 実行する操作は plain `git push` のみ。force / upstream 設定 / tag push などの
// 追加引数は UI からもサーバからも付与しない。
func (s *Server) handleGitPush(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req gitPushReq
	if !decodeJSON(w, r, &req) {
		return
	}
	sid := req.Session
	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), gitPushTimeout)
	defer cancel()
	out, runErr := runGitCombinedEnv(ctx, cwd, gitPushNonInteractiveEnv(), "push")
	if runErr != nil {
		code, status := classifyGitPushError(string(out))
		s.logger.Warn("git push failed", "session_id", sid, "err", runErr, "output", string(out))
		writeGitError(w, status, code, sanitizeGitErrMsg(runErr))
		return
	}
	writeJSON(w, gitPushResp{OK: true, Output: string(out)})
}

// classifyGitPushError は plain `git push` の combined output から
// エラー種別コードと HTTP ステータスを判定する。
func classifyGitPushError(out string) (code string, status int) {
	msg := strings.ToLower(out)
	switch {
	case strings.Contains(msg, "non-fast-forward"),
		strings.Contains(msg, "fetch first"),
		strings.Contains(msg, "rejected"):
		return "rejected_non_fast_forward", http.StatusConflict
	case strings.Contains(msg, "no upstream"),
		strings.Contains(msg, "no configured push destination"):
		return "no_upstream", http.StatusBadRequest
	case strings.Contains(msg, "authentication failed"),
		strings.Contains(msg, "could not read username"),
		strings.Contains(msg, "permission denied"),
		strings.Contains(msg, "publickey"):
		return "auth_failed", http.StatusBadGateway
	default:
		return "git_command_failed", http.StatusInternalServerError
	}
}

package hub

import (
	"context"
	"net/http"
	"time"
)

const gitFetchTimeout = 30 * time.Second

type gitFetchReq struct {
	Session int    `json:"session"`
	Token   string `json:"token"`
}

type gitFetchResp struct {
	OK     bool   `json:"ok"`
	Output string `json:"output,omitempty"`
}

// handleGitFetch は POST /api/git-fetch を処理する。
// リモートから最新を取得する（git fetch）。
func (s *Server) handleGitFetch(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req gitFetchReq
	if !decodeJSON(w, r, &req) {
		return
	}
	sid := req.Session
	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), gitFetchTimeout)
	defer cancel()
	out, runErr := runGitCombined(ctx, cwd, "fetch")
	if runErr != nil {
		s.logger.Warn("git fetch failed", "session_id", sid, "err", runErr, "output", string(out))
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(runErr))
		return
	}
	writeJSON(w, gitFetchResp{OK: true, Output: string(out)})
}

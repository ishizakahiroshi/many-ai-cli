package hub

import (
	"context"
	"encoding/json"
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
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req gitFetchReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeGitError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	if req.Token != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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
	w.Header().Set("Content-Type", "application/json")
	if runErr != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", runErr.Error())
		return
	}
	_ = json.NewEncoder(w).Encode(gitFetchResp{OK: true, Output: string(out)})
}

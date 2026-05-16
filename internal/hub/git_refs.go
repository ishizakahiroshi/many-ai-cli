package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
)

// gitRefsResp は /api/git-refs のレスポンス。
type gitRefsResp struct {
	OK        bool     `json:"ok"`
	Head      string   `json:"head"`
	Refs      []gitRef `json:"refs"`
	GithubURL string   `json:"github_url,omitempty"`
}

// handleGitRefs は GET /api/git-refs を処理する。
// クエリ: session, token
func (s *Server) handleGitRefs(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}

	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	// 1) for-each-ref で全 ref 列挙
	out, err := runGit(ctx, cwd, "for-each-ref",
		"--format=%(refname)%09%(objectname:short)")
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
		return
	}
	refs := parseForEachRef(string(out))

	// 2) HEAD: symbolic-ref --short HEAD（失敗時は detached:<short>）
	head := ""
	if symOut, serr := runGit(ctx, cwd, "symbolic-ref", "--short", "HEAD"); serr == nil {
		head = strings.TrimSpace(string(symOut))
	} else {
		if shortOut, sherr := runGit(ctx, cwd, "rev-parse", "--short", "HEAD"); sherr == nil {
			short := strings.TrimSpace(string(shortOut))
			if short != "" {
				head = "detached:" + short
			}
		}
	}

	resp := gitRefsResp{
		OK:   true,
		Head: head,
		Refs: refs,
	}

	// 3) origin URL から github.com の場合のみ github_url を埋める（C3 mock 拡張用）
	if originOut, oerr := runGit(ctx, cwd, "remote", "get-url", "origin"); oerr == nil {
		if g := toGithubHTTPSURL(strings.TrimSpace(string(originOut))); g != "" {
			resp.GithubURL = g
		}
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

// parseForEachRef は `git for-each-ref --format="%(refname)\t%(objectname:short)"` 出力をパースする。
// 各行: `refs/heads/develop\t44c44044`
//
// 分類:
//   - refs/heads/X    → {kind: "local",  name: "X"}
//   - refs/remotes/X  → {kind: "remote", name: "X"}（origin/HEAD は除外）
//   - refs/tags/X     → {kind: "tag",    name: "X"}
//   - その他          → 無視
func parseForEachRef(raw string) []gitRef {
	refs := []gitRef{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 2)
		if len(parts) < 2 {
			continue
		}
		refname := parts[0]
		hash := parts[1]
		switch {
		case strings.HasPrefix(refname, "refs/heads/"):
			name := strings.TrimPrefix(refname, "refs/heads/")
			if name == "" {
				continue
			}
			refs = append(refs, gitRef{Kind: "local", Name: name, Hash: hash})
		case strings.HasPrefix(refname, "refs/remotes/"):
			name := strings.TrimPrefix(refname, "refs/remotes/")
			if name == "" || strings.HasSuffix(name, "/HEAD") {
				continue
			}
			refs = append(refs, gitRef{Kind: "remote", Name: name, Hash: hash})
		case strings.HasPrefix(refname, "refs/tags/"):
			name := strings.TrimPrefix(refname, "refs/tags/")
			if name == "" {
				continue
			}
			refs = append(refs, gitRef{Kind: "tag", Name: name, Hash: hash})
		}
	}
	return refs
}

// toGithubHTTPSURL は origin の URL を GitHub HTTPS 形式に正規化する。
// 対応形式:
//   - git@github.com:owner/repo.git    → https://github.com/owner/repo
//   - https://github.com/owner/repo.git → https://github.com/owner/repo
//   - https://github.com/owner/repo    → https://github.com/owner/repo
//   - ssh://git@github.com/owner/repo.git → https://github.com/owner/repo
//
// github.com 以外（GitLab 等）は空文字を返す。
func toGithubHTTPSURL(remote string) string {
	if remote == "" {
		return ""
	}
	// SCP-like: git@github.com:owner/repo.git
	if strings.HasPrefix(remote, "git@") {
		// 想定: git@<host>:<path>
		colon := strings.Index(remote, ":")
		at := strings.Index(remote, "@")
		if colon < 0 || at < 0 || colon < at {
			return ""
		}
		host := remote[at+1 : colon]
		path := remote[colon+1:]
		if host != "github.com" {
			return ""
		}
		path = strings.TrimSuffix(path, ".git")
		path = strings.TrimPrefix(path, "/")
		if path == "" {
			return ""
		}
		return "https://github.com/" + path
	}
	// http(s):// or ssh://
	u, err := url.Parse(remote)
	if err != nil || u.Host == "" {
		return ""
	}
	host := u.Host
	// ssh://git@github.com:22/... のようなポート付きを許容
	if idx := strings.Index(host, ":"); idx >= 0 {
		host = host[:idx]
	}
	if host != "github.com" {
		return ""
	}
	path := strings.TrimSuffix(u.Path, ".git")
	path = strings.TrimPrefix(path, "/")
	if path == "" {
		return ""
	}
	return "https://github.com/" + path
}

package hub

import (
	"context"
	"net/http"
	"strconv"
	"strings"
)

// gitLogCommit は /api/git-log の各 commit エントリ。
type gitLogCommit struct {
	Hash        string   `json:"hash"`
	ShortHash   string   `json:"short_hash"`
	Parents     []string `json:"parents"`
	AuthorName  string   `json:"author_name"`
	AuthorEmail string   `json:"author_email"`
	AuthorDate  string   `json:"author_date"`
	Subject     string   `json:"subject"`
	Refs        []gitRef `json:"refs"`
}

// gitLogResp は /api/git-log のレスポンス。
type gitLogResp struct {
	OK       bool           `json:"ok"`
	GitRoot  string         `json:"git_root"`
	Branch   string         `json:"branch"`
	HeadHash string         `json:"head_hash"`
	Commits  []gitLogCommit `json:"commits"`
	Limit    int            `json:"limit"`
	Skip     int            `json:"skip"`
	HasMore  bool           `json:"has_more"`
}

const (
	gitLogDefaultLimit = 100
	gitLogMaxLimit     = 1000
	// pretty format に揃えるためのフィールド数:
	// %H \t %h \t %P \t %an \t %ae \t %aI \t %D \t %s
	gitLogPrettyFormat = "%H%x09%h%x09%P%x09%an%x09%ae%x09%aI%x09%D%x09%s"
	gitLogFieldCount   = 8
)

// handleGitLog は GET /api/git-log を処理する。
// クエリ: session, token, ref (default=HEAD), limit (default=100), skip (default=0)
// ref=--all は `git log --all` として扱う。
func (s *Server) handleGitLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	q := r.URL.Query()
	sid, ok := parseSessionID(q.Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	ref := strings.TrimSpace(q.Get("ref"))
	if ref == "" {
		ref = "HEAD"
	}
	// ref=--all は特別扱い（git log --all）。それ以外は revision 形式チェック。
	if ref != "--all" && !validRevision(ref) {
		writeGitError(w, http.StatusBadRequest, "bad_request", "invalid ref format")
		return
	}
	limit := gitLogDefaultLimit
	if v := q.Get("limit"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			if n > gitLogMaxLimit {
				n = gitLogMaxLimit
			}
			limit = n
		}
	}
	skip := 0
	if v := q.Get("skip"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n >= 0 {
			skip = n
		}
	}

	gitRoot, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	// HEAD hash / 現在 branch（detach 含む）
	headHash := ""
	if out, herr := runGit(ctx, cwd, "rev-parse", "HEAD"); herr == nil {
		headHash = strings.TrimSpace(string(out))
	}
	branch := ""
	if out, berr := runGit(ctx, cwd, "rev-parse", "--abbrev-ref", "HEAD"); berr == nil {
		branch = strings.TrimSpace(string(out))
		if branch == "HEAD" {
			if shortOut, sherr := runGit(ctx, cwd, "rev-parse", "--short", "HEAD"); sherr == nil {
				branch = "detached:" + strings.TrimSpace(string(shortOut))
			}
		}
	}

	// git log 実行（-- で revision とファイルパス境界を明示）
	args := []string{"log"}
	if ref == "--all" {
		args = append(args, "--all")
	} else {
		// -- を前に置いて ref をオプションと誤解させない
		args = append(args, ref, "--")
	}
	args = append(args,
		"--max-count="+strconv.Itoa(limit),
		"--skip="+strconv.Itoa(skip),
		"--date-order",
		"--decorate=short",
		"--pretty=format:"+gitLogPrettyFormat,
	)
	out, err := runGit(ctx, cwd, args...)
	if err != nil {
		s.logger.Warn("git log failed", "session_id", sid, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}

	commits := parseGitLog(string(out))

	// has_more 判定: rev-list --count <ref> で総数取得
	hasMore := false
	countArgs := []string{"rev-list", "--count"}
	if ref == "--all" {
		countArgs = append(countArgs, "--all")
	} else {
		countArgs = append(countArgs, ref, "--")
	}
	if cntOut, cerr := runGit(ctx, cwd, countArgs...); cerr == nil {
		if total, perr := strconv.Atoi(strings.TrimSpace(string(cntOut))); perr == nil {
			hasMore = (skip + len(commits)) < total
		}
	}

	resp := gitLogResp{
		OK:       true,
		GitRoot:  gitRoot,
		Branch:   branch,
		HeadHash: headHash,
		Commits:  commits,
		Limit:    limit,
		Skip:     skip,
		HasMore:  hasMore,
	}
	writeJSON(w, resp)
}

// parseGitLog は gitLogPrettyFormat 出力をパースして gitLogCommit スライスを返す。
// 出力 1 行 = 1 commit。
func parseGitLog(raw string) []gitLogCommit {
	if raw == "" {
		return []gitLogCommit{}
	}
	lines := strings.Split(raw, "\n")
	commits := make([]gitLogCommit, 0, len(lines))
	for _, line := range lines {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", gitLogFieldCount)
		if len(parts) < gitLogFieldCount {
			// 想定外フォーマット: スキップ
			continue
		}
		parents := []string{}
		if pf := strings.TrimSpace(parts[2]); pf != "" {
			parents = strings.Fields(pf)
		}
		commits = append(commits, gitLogCommit{
			Hash:        parts[0],
			ShortHash:   parts[1],
			Parents:     parents,
			AuthorName:  parts[3],
			AuthorEmail: parts[4],
			AuthorDate:  parts[5],
			Refs:        parseDecorate(parts[6]),
			Subject:     parts[7],
		})
	}
	return commits
}

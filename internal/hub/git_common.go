package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// gitCommandTimeout は /api/git-* で発行する全 git コマンドの上限時間。
// 5s 以内に返らない場合はキャンセルされ git_command_failed を返す。
const gitCommandTimeout = 5 * time.Second

// gitErrorResp は git API の共通エラー JSON 形式。
//
//	{"ok": false, "error": "<code>", "detail": "<msg>"}
type gitErrorResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error"`
	Detail string `json:"detail,omitempty"`
}

// writeGitError は git API のエラーレスポンスを書き出す。
// status は HTTP ステータスコード（400/404/500 等）。
func writeGitError(w http.ResponseWriter, status int, code, detail string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(gitErrorResp{
		OK:     false,
		Error:  code,
		Detail: detail,
	})
}

// parseSessionID は ?session= クエリを int に変換する。
// 空・不正値の場合は (0, false) を返す。
func parseSessionID(raw string) (int, bool) {
	if strings.TrimSpace(raw) == "" {
		return 0, false
	}
	sid, err := strconv.Atoi(raw)
	if err != nil {
		return 0, false
	}
	return sid, true
}

// resolveGitRoot は session ID からセッション cwd を取り出し、
// `git -C <cwd> rev-parse --show-toplevel` で git root を確定する。
//
// 戻り値:
//   - gitRoot: 絶対パス
//   - cwd:     セッションの作業ディレクトリ（git -C 引数に使う元の値）
//   - err:     bad_session / no_cwd / not_git_repo / git_command_failed の判別用
//
// err は内部判別用に標準 error を返す。呼び出し側で errors.Is で分類する。
func (s *Server) resolveGitRoot(sid int) (gitRoot, cwd string, err error) {
	s.mu.Lock()
	ses := s.sessions[sid]
	if ses == nil {
		s.mu.Unlock()
		return "", "", errBadSession
	}
	cwd = ses.CWD
	s.mu.Unlock()

	if strings.TrimSpace(cwd) == "" {
		return "", "", errNoCWD
	}

	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	out, runErr := runGit(ctx, cwd, "rev-parse", "--show-toplevel")
	if runErr != nil {
		// rev-parse 失敗は git リポジトリでないとみなす（git 自体が無い場合も含むが、
		// その場合はクライアント側で git 未インストール扱いとして同じプレースホルダで対応）
		return "", cwd, fmt.Errorf("%w: %v", errNotGitRepo, runErr)
	}
	gitRoot = strings.TrimSpace(string(out))
	if gitRoot == "" {
		return "", cwd, errNotGitRepo
	}
	return gitRoot, cwd, nil
}

var (
	errBadSession       = errors.New("bad_session")
	errNoCWD            = errors.New("no_cwd")
	errNotGitRepo       = errors.New("not_git_repo")
	errCommitIdentity   = errors.New("commit_identity_missing")
	errNoChanges        = errors.New("no_changes")
	errBadCommitMessage = errors.New("bad_commit_message")
)

// runGit は `git -C <cwd> <args...>` を実行し stdout を返す。
// stderr は ExitError から拾ってエラーメッセージに含める（呼び出し側のロギング用）。
// ctx の timeout / cancel で確実に終了する。
func runGit(ctx context.Context, cwd string, args ...string) ([]byte, error) {
	full := append([]string{"-C", cwd}, args...)
	cmd := exec.CommandContext(ctx, "git", full...)
	out, err := cmd.Output()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && len(exitErr.Stderr) > 0 {
			return out, fmt.Errorf("git %s: %s", strings.Join(args, " "),
				strings.TrimSpace(string(exitErr.Stderr)))
		}
		return out, fmt.Errorf("git %s: %w", strings.Join(args, " "), err)
	}
	return out, nil
}

// runGitCombined は stdout/stderr をまとめて返す。commit のように失敗理由が stderr
// に出るコマンドで、UI に見せる detail を失わないために使う。
func runGitCombined(ctx context.Context, cwd string, args ...string) ([]byte, error) {
	full := append([]string{"-C", cwd}, args...)
	cmd := exec.CommandContext(ctx, "git", full...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return out, fmt.Errorf("git %s: %s", strings.Join(args, " "), strings.TrimSpace(string(out)))
	}
	return out, nil
}

func sanitizeCommitMessage(s string, maxLen int) string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	s = strings.TrimSpace(s)
	if maxLen > 0 && len(s) > maxLen {
		s = s[:maxLen]
		s = strings.TrimSpace(s)
	}
	return s
}

func classifyGitCommitError(err error) (code string, status int) {
	if err == nil {
		return "", http.StatusOK
	}
	msg := strings.ToLower(err.Error())
	switch {
	case errors.Is(err, errCommitIdentity),
		strings.Contains(msg, "author identity unknown"),
		strings.Contains(msg, "please tell me who you are"),
		strings.Contains(msg, "unable to auto-detect email address"):
		return "commit_identity_missing", http.StatusBadRequest
	case errors.Is(err, errNoChanges):
		return "no_changes", http.StatusBadRequest
	case strings.Contains(msg, "nothing to commit"):
		return "no_changes", http.StatusBadRequest
	case errors.Is(err, errBadCommitMessage):
		return "bad_request", http.StatusBadRequest
	default:
		return "git_command_failed", http.StatusInternalServerError
	}
}

// gitRef は decorate / for-each-ref / for git-log・git-show・git-refs 共通の ref エントリ。
type gitRef struct {
	Kind string `json:"kind"`           // "local" | "remote" | "tag" | "head"
	Name string `json:"name"`           // local: "develop", remote: "origin/develop", tag: "v0.1.3"
	Hash string `json:"hash,omitempty"` // for-each-ref 用（log/show では省略）
}

// parseDecorate は `%D` (refs decorate) を gitRef スライスに変換する。
//
// 例:
//
//	"HEAD -> develop, origin/develop, tag: v0.1.3"
//	→ [{local, develop}, {remote, origin/develop}, {tag, v0.1.3}]
//
// "HEAD" / "HEAD -> X" の HEAD 部分はスキップ（head_hash は別フィールド）。
func parseDecorate(decorate string) []gitRef {
	decorate = strings.TrimSpace(decorate)
	if decorate == "" {
		return nil
	}
	parts := strings.Split(decorate, ",")
	refs := make([]gitRef, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p == "" {
			continue
		}
		// "HEAD -> develop" の HEAD ポインタ部分を剥がす
		if idx := strings.Index(p, "->"); idx >= 0 {
			p = strings.TrimSpace(p[idx+2:])
		}
		if p == "" || p == "HEAD" {
			continue
		}
		switch {
		case strings.HasPrefix(p, "tag:"):
			name := strings.TrimSpace(strings.TrimPrefix(p, "tag:"))
			if name != "" {
				refs = append(refs, gitRef{Kind: "tag", Name: name})
			}
		case strings.HasPrefix(p, "origin/") || strings.Contains(p, "/"):
			// remote 名は "<remote>/<branch>" 形式
			refs = append(refs, gitRef{Kind: "remote", Name: p})
		default:
			refs = append(refs, gitRef{Kind: "local", Name: p})
		}
	}
	return refs
}

// writeGitErrorFromResolve は resolveGitRoot が返したエラーを適切な JSON エラーに変換する。
// sid を detail に含めることで「どのセッションが見つからなかったか」を UI / ログから追える。
func writeGitErrorFromResolve(w http.ResponseWriter, sid int, err error) {
	switch {
	case errors.Is(err, errBadSession):
		writeGitError(w, http.StatusBadRequest, "bad_session", fmt.Sprintf("session not found (sid=%d)", sid))
	case errors.Is(err, errNoCWD):
		writeGitError(w, http.StatusBadRequest, "no_cwd", fmt.Sprintf("session has no cwd (sid=%d)", sid))
	case errors.Is(err, errNotGitRepo):
		writeGitError(w, http.StatusBadRequest, "not_git_repo", err.Error())
	default:
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
	}
}

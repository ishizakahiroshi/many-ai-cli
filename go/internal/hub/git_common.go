package hub

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// revPattern は git revision（hash / ブランチ名 / タグ名）として許可する文字を定義する。
// 英数字・ドット・スラッシュ・アンダースコア・ハイフンのみ許可。
// 先頭 "-" はオプションと紛らわしいため rejectedする。
var revPattern = regexp.MustCompile(`^[0-9A-Za-z._/\-]+$`)

// validRevision は s が git revision として安全かを検証する。
// 空文字・先頭 "-"・許可外文字があれば false を返す。
func validRevision(s string) bool {
	return s != "" && !strings.HasPrefix(s, "-") && revPattern.MatchString(s)
}

// gitCmdError は git の実行失敗を「引数」と「出力」に分けて保持する。
// Error() は従来どおり "git <args>: <output>" を返すので、classifyGitCommitError の
// ような文字列判定はそのまま動く。
//
// 分けて持つ理由は sanitizeGitErrMsg のため。従来は 1 本の文字列を最初の ": " で
// 割っていたが、commit のように引数へ自由入力（コミットメッセージ）が入る場合、
// メッセージ中の ": " で割れて出力側が丸ごと落ちていた。実例として
// `git commit -m feat: <件名>` は "git commit -m feat" と "<件名>…" に割れ、
// pre-commit hook が書いた拒否理由が UI から消えていた（2026-09-21 実測・
// docs/local/bugfix_git-hook-rejection-reason-hidden_2026-09-21.md）。
type gitCmdError struct {
	Args   string
	Output string
}

func (e *gitCmdError) Error() string {
	if e.Output == "" {
		return "git " + e.Args
	}
	return "git " + e.Args + ": " + e.Output
}

// 伏字の置き換え先。スラッシュを含めないこと（「絶対パスが残っていないか」を
// スラッシュの有無で見る検査があるため）。
const (
	gitErrRedactedPath = "<path>"
	gitErrRedactedURL  = "<url>"
)

// UI へ返すエラー detail の上限。git の出力（とくに hook の出力）は長い。
// 全文は呼び出し元が slog へ出すので、画面には頭だけ出して導線を添える。
const (
	gitErrMaxArgsBytes   = 120
	gitErrMaxOutputBytes = 900
	gitErrMaxOutputLines = 12
)

const gitErrTruncatedNote = " … (truncated; full output is in the Hub log)"

var (
	// redactURLPattern: scheme://… をまとめて伏せる。認証情報付きの remote URL
	// （https://<token>@host/…）を UI へ出さないため。他の伏字より先に当てる。
	redactURLPattern = regexp.MustCompile(`[A-Za-z][A-Za-z0-9+.\-]*://[^\s"'<>|]*`)
	// 以下は「絶対パス」だけを伏せる。リポジトリ相対パス（web/src/app/foo.ts）は
	// 公開リポに載っている情報なので伏せない。**ここを伏せると hook の拒否理由が
	// 消える**（hook が書くのはほぼ相対パスだけ）。伏せたいのは利用者名やマシン構成が
	// 出る絶対パスのほう。
	redactUNCPattern     = regexp.MustCompile(`\\\\[^\s"'<>|]+`)
	redactWinAbsPattern  = regexp.MustCompile(`[A-Za-z]:[\\/][^\s"'<>|]*`)
	redactUnixAbsPattern = regexp.MustCompile(`(^|[\s"'(\[=:,])/[^\s"'<>|)\]:]*`)
)

// redactGitPaths は絶対パスとリモート URL だけを伏字へ置き換える。
// 相対パスと説明文はそのまま残す。
func redactGitPaths(s string) string {
	s = redactURLPattern.ReplaceAllString(s, gitErrRedactedURL)
	s = redactUNCPattern.ReplaceAllString(s, gitErrRedactedPath)
	s = redactWinAbsPattern.ReplaceAllString(s, gitErrRedactedPath)
	s = redactUnixAbsPattern.ReplaceAllString(s, "${1}"+gitErrRedactedPath)
	return s
}

// capGitErrText は行数とバイト数で頭を切り、切ったときだけ導線を添える。
func capGitErrText(s string, maxBytes, maxLines int) string {
	s = strings.TrimSpace(s)
	truncated := false
	if maxLines > 0 {
		if lines := strings.Split(s, "\n"); len(lines) > maxLines {
			s = strings.Join(lines[:maxLines], "\n")
			truncated = true
		}
	}
	if maxBytes > 0 && len(s) > maxBytes {
		s = truncateUTF8Bytes(s, maxBytes)
		truncated = true
	}
	if truncated {
		return strings.TrimSpace(s) + gitErrTruncatedNote
	}
	return s
}

// sanitizeGitErrMsg は git コマンドエラーから UI に返す文字列を生成する。
//
// **失敗した理由は残す。** 伏せるのは絶対パスとリモート URL だけで、git や hook が
// 書いた説明文とリポジトリ相対パスはそのまま通す。以前はパスらしき文字が 1 つでも
// あれば出力を丸ごと "git command failed" に差し替えていたが、pre-commit hook の
// 出力はほぼ必ずパスを含むので、**hook に止められた理由が 100% 消えていた**
// （docs/local/bugfix_git-hook-rejection-reason-hidden_2026-09-21.md）。
//
// 全文は呼び出し元が slog へ記録する。ここが返すのは画面に載せる頭だけ。
func sanitizeGitErrMsg(err error) string {
	if err == nil {
		return ""
	}
	var cmdErr *gitCmdError
	if errors.As(err, &cmdErr) {
		args := capGitErrText(redactGitPaths(cmdErr.Args), gitErrMaxArgsBytes, 1)
		out := capGitErrText(redactGitPaths(cmdErr.Output), gitErrMaxOutputBytes, gitErrMaxOutputLines)
		if out == "" {
			return strings.TrimSpace("git " + args)
		}
		return strings.TrimSpace("git "+args) + ": " + out
	}
	// 構造を持たないエラー（sentinel・context のキャンセル等）は文字列のまま伏字にする。
	msg := capGitErrText(redactGitPaths(err.Error()), gitErrMaxOutputBytes, gitErrMaxOutputLines)
	if msg == "" {
		return "git command failed"
	}
	return msg
}

// gitCommandTimeout は /api/git-* で発行する全 git コマンドの上限時間。
// 5s 以内に返らない場合はキャンセルされ git_command_failed を返す。
const gitCommandTimeout = 5 * time.Second

// writeGitError は git API のエラーレスポンスを書き出す。
// status は HTTP ステータスコード（400/404/500 等）。
func writeGitError(w http.ResponseWriter, status int, code, detail string) {
	writeJSONError(w, status, code, detail)
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
	s.sessionsMu.Lock()
	ses := s.sessions[sid]
	if ses == nil {
		s.sessionsMu.Unlock()
		return "", "", errBadSession
	}
	cwd = ses.CWD
	s.sessionsMu.Unlock()

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
	// core.quotePath=false: keep non-ASCII (e.g. Japanese) paths as raw UTF-8 in
	// name-status / numstat output instead of octal-escaped quoted form. Quoted
	// paths break both the UI display and pathspec reuse (`git show <hash> -- "\343..."`
	// matches nothing and returns an empty, exit-0 diff). -z commands are
	// unaffected (they never quote).
	full := append([]string{"-C", cwd, "-c", "core.quotePath=false"}, args...)
	// #nosec G702 -- argv 直渡しで shell を介さないので、シェル注入は成立しない。
	// 残る危険は「引数がオプションに化ける」形（--upload-pack= 等）だが、可変値を
	// 渡す呼び出しは全て手前で塞いである。2026-08-28 に全呼び出しを確認した:
	//   - git_log.go の ref と git_show.go の hash は validRevision() を通る。
	//     同関数は先頭 "-" を明示的に拒否する（このファイルの validRevision）
	//   - git_diff.go / git_show.go のファイルパスは "--" より後ろに置く
	//   - それ以外の引数はコード中のリテラル
	//   - cwd は自ホストの session cwd であって外部入力ではない
	// gosec の taint 解析はこの検査を追えず、しかも判定が run ごとに揺れる
	// （同一コミットで 1 件 / 2 件が交互に出るのを 2026-08-28 に実測）。
	// 抑制して CI の結果を決定的にする。呼び出しを増やすときは上の 4 条件を守ること。
	cmd := exec.CommandContext(ctx, "git", full...)
	out, err := cmd.Output()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && len(exitErr.Stderr) > 0 {
			return out, &gitCmdError{
				Args:   strings.Join(args, " "),
				Output: strings.TrimSpace(string(exitErr.Stderr)),
			}
		}
		return out, fmt.Errorf("git %s: %w", strings.Join(args, " "), err)
	}
	return out, nil
}

// runGitCombined は stdout/stderr をまとめて返す。commit のように失敗理由が stderr
// に出るコマンドで、UI に見せる detail を失わないために使う。
func runGitCombined(ctx context.Context, cwd string, args ...string) ([]byte, error) {
	return runGitCombinedEnv(ctx, cwd, nil, args...)
}

// runGitCombinedEnv は stdout/stderr をまとめて返しつつ、必要な追加環境変数を
// git プロセスへ渡す。push のように認証プロンプトを抑止したい場合に使う。
func runGitCombinedEnv(ctx context.Context, cwd string, extraEnv []string, args ...string) ([]byte, error) {
	// See runGit: keep non-ASCII paths raw so parsers and pathspec reuse work.
	full := append([]string{"-C", cwd, "-c", "core.quotePath=false"}, args...)
	cmd := exec.CommandContext(ctx, "git", full...)
	if len(extraEnv) > 0 {
		cmd.Env = append(os.Environ(), extraEnv...)
	}
	out, err := cmd.CombinedOutput()
	if err != nil {
		return out, &gitCmdError{
			Args:   strings.Join(args, " "),
			Output: strings.TrimSpace(string(out)),
		}
	}
	return out, nil
}

func sanitizeCommitMessage(s string, maxLen int) string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	// \t / \n 以外の C0 制御文字と DEL を除去する。BEL/ESC 等が残ると git 履歴や
	// WS 配信に混入し、ターミナルで git log を見た第三者にエスケープシーケンス
	// （タイトルバー詐称・画面クリア等）を注入できてしまうため。
	s = strings.Map(func(r rune) rune {
		if (r < 0x20 && r != '\t' && r != '\n') || r == 0x7f {
			return -1
		}
		return r
	}, s)
	s = strings.TrimSpace(s)
	if maxLen > 0 && len(s) > maxLen {
		// rune 境界で丸める（バイト単位の切り詰めはマルチバイト文字を分断し
		// 不正 UTF-8 を git 履歴へ永続化させるため、既存の rune 安全ヘルパを使う）。
		s = truncateUTF8Bytes(s, maxLen)
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
// 例（短縮表示）:
//
//	"HEAD -> develop, origin/develop, tag: v0.1.3"
//	→ [{local, develop}, {remote, origin/develop}, {tag, v0.1.3}]
//
// "HEAD" / "HEAD -> X" の HEAD 部分はスキップ（head_hash は別フィールド）。
// git log/show は --decorate=full を使うため、refs/heads・refs/remotes・
// refs/tags の種別を優先する。短縮表示で slash を含む名前は種別を復元できない
// ので local とし、従来の origin/ だけは後方互換で remote として扱う。
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
		case strings.HasPrefix(p, "refs/heads/"):
			name := strings.TrimPrefix(p, "refs/heads/")
			if name != "" {
				refs = append(refs, gitRef{Kind: "local", Name: name})
			}
		case strings.HasPrefix(p, "refs/remotes/"):
			name := strings.TrimPrefix(p, "refs/remotes/")
			if name != "" {
				refs = append(refs, gitRef{Kind: "remote", Name: name})
			}
		case strings.HasPrefix(p, "refs/tags/"):
			name := strings.TrimPrefix(p, "refs/tags/")
			if name != "" {
				refs = append(refs, gitRef{Kind: "tag", Name: name})
			}
		case strings.HasPrefix(p, "tag:"):
			name := strings.TrimSpace(strings.TrimPrefix(p, "tag:"))
			name = strings.TrimPrefix(name, "refs/tags/")
			if name != "" {
				refs = append(refs, gitRef{Kind: "tag", Name: name})
			}
		case strings.HasPrefix(p, "origin/"):
			// Backward-compatible short decoration for the conventional remote.
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
		writeGitError(w, http.StatusBadRequest, "not_git_repo", sanitizeGitErrMsg(err))
	default:
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
	}
}

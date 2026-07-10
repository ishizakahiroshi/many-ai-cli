package hub

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// gitDiffResp は /api/git-diff のレスポンス。
// 作業ツリー（staged + unstaged + untracked）と HEAD の per-file diff を返す。
type gitDiffResp struct {
	OK       bool             `json:"ok"`
	GitRoot  string           `json:"git_root"`
	RepoName string           `json:"repo_name"`
	Branch   string           `json:"branch"`
	HeadHash string           `json:"head_hash"`
	Files    []gitShowFile    `json:"files"`
	Summary  gitStatusSummary `json:"summary"`
}

// binaryProbeBytes は合成 diff のバイナリ判定で調べる先頭バイト数。
// git 本体の判定（先頭 8000 バイトに NUL があればバイナリ）に合わせる。
const binaryProbeBytes = 8000

// handleGitDiff は GET /api/git-diff を処理する（Review タブ Phase 1）。
// クエリ: session, token
//
// tracked ファイルは `git diff HEAD -- <path>`、untracked ファイルは
// `git diff --no-index` の null デバイス指定が OS 依存（NUL / /dev/null）なため
// Go 側でファイル内容から「新規追加」の unified diff を合成する。
func (s *Server) handleGitDiff(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	// untracked をディレクトリ丸めせずファイル単位で列挙する（-uall）
	statusOut, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1", "-z", "--untracked-files=all")
	if err != nil {
		s.logger.Warn("git diff status failed", "session_id", sid, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	statusFiles := parseGitStatusPorcelainZ(string(statusOut))

	// HEAD が無いリポジトリ（コミット 0）では tracked 比較先が無いので
	// 全ファイルを合成 diff で返す。
	headHash := ""
	if out, herr := runGit(ctx, cwd, "rev-parse", "HEAD"); herr == nil {
		headHash = strings.TrimSpace(string(out))
	}

	numstats := map[string]numstatEntry{}
	if headHash != "" {
		if out, nerr := runGit(ctx, cwd, "diff", "--numstat", "HEAD", "--"); nerr == nil {
			numstats = parseNumstatRaw(string(out))
		}
	}

	files := make([]gitShowFile, 0, len(statusFiles))
	for _, sf := range statusFiles {
		f := gitShowFile{Status: sf.Status, Path: sf.Path}
		untracked := sf.Status == "??"
		if untracked {
			f.Status = "A"
		}
		if untracked || headHash == "" {
			diff, added := synthesizeUntrackedDiff(gitRoot, sf.Path)
			f.Diff = diff
			f.Added = added
		} else {
			if v, ok := numstats[sf.Path]; ok {
				f.Added = v.added
				f.Removed = v.removed
			}
			diffOut, derr := runGit(ctx, cwd, "diff", "HEAD", "--", sf.Path)
			if derr == nil {
				diff := strings.TrimLeft(string(diffOut), "\n")
				if len(diff) > gitShowDiffMaxBytes {
					diff = diff[:gitShowDiffMaxBytes] + "\n(truncated)"
				}
				f.Diff = diff
			}
			// 個別エラーは diff 空のまま返す（git-show と同じ方針）
		}
		files = append(files, f)
	}

	summary := gitStatusSummary{FilesChanged: len(files)}
	for _, f := range files {
		summary.Added += f.Added
		summary.Removed += f.Removed
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

	writeJSON(w, gitDiffResp{
		OK:       true,
		GitRoot:  gitRoot,
		RepoName: filepath.Base(gitRoot),
		Branch:   branch,
		HeadHash: headHash,
		Files:    files,
		Summary:  summary,
	})
}

// synthesizeUntrackedDiff は untracked ファイルを読み、新規追加の unified diff を合成する。
// porcelain の path は repo root 相対なので gitRoot と結合する。
// 読めないファイル（削除競合・権限等）は diff 空・追加 0 行で返す。
func synthesizeUntrackedDiff(gitRoot, relPath string) (string, int) {
	abs := filepath.Join(gitRoot, filepath.FromSlash(relPath))
	data, err := os.ReadFile(abs)
	if err != nil {
		return "", 0
	}
	return synthesizeAddDiff(relPath, data)
}

// synthesizeAddDiff は content 全体を「新規ファイル追加」の unified diff にする。
// バイナリ（先頭 binaryProbeBytes バイトに NUL を含む）はプレースホルダ 1 行のみ。
// 出力が gitShowDiffMaxBytes を超える場合は行単位で打ち切り "(truncated)" を付す。
// 戻り値は (diff, 追加行数)。追加行数は numstat 相当（バイナリは 0）。
func synthesizeAddDiff(relPath string, data []byte) (string, int) {
	header := "diff --git a/" + relPath + " b/" + relPath + "\n" +
		"new file mode 100644\n" +
		"--- /dev/null\n" +
		"+++ b/" + relPath + "\n"
	probe := data
	if len(probe) > binaryProbeBytes {
		probe = probe[:binaryProbeBytes]
	}
	if bytes.IndexByte(probe, 0) >= 0 {
		return header + "Binary file (new)\n", 0
	}
	if len(data) == 0 {
		return header, 0
	}
	content := string(data)
	noEOFNewline := !strings.HasSuffix(content, "\n")
	content = strings.TrimSuffix(content, "\n")
	lines := strings.Split(content, "\n")

	var b strings.Builder
	b.WriteString(header)
	fmt.Fprintf(&b, "@@ -0,0 +1,%d @@\n", len(lines))
	truncated := false
	for _, ln := range lines {
		if b.Len() > gitShowDiffMaxBytes {
			truncated = true
			break
		}
		b.WriteString("+")
		b.WriteString(ln)
		b.WriteString("\n")
	}
	switch {
	case truncated:
		b.WriteString("(truncated)\n")
	case noEOFNewline:
		b.WriteString("\\ No newline at end of file\n")
	}
	return b.String(), len(lines)
}

package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
)

// gitShowFile は /api/git-show の各ファイル diff エントリ。
type gitShowFile struct {
	Status  string `json:"status"`  // A / M / D / R / C / T 等
	Path    string `json:"path"`
	Added   int    `json:"added"`
	Removed int    `json:"removed"`
	Diff    string `json:"diff"`
}

// gitShowResp は /api/git-show のレスポンス。
type gitShowResp struct {
	OK          bool          `json:"ok"`
	Hash        string        `json:"hash"`
	Parents     []string      `json:"parents"`
	AuthorName  string        `json:"author_name"`
	AuthorEmail string        `json:"author_email"`
	AuthorDate  string        `json:"author_date"`
	Subject     string        `json:"subject"`
	Body        string        `json:"body"`
	Refs        []gitRef      `json:"refs"`
	Files       []gitShowFile `json:"files"`
}

const (
	// 1 ファイル diff の上限。超えたら "\n(truncated)" を末尾に付与。
	gitShowDiffMaxBytes = 256 * 1024
	// メタ取得の pretty format。subject と body は \x1f (US) で分離する。
	// %H \t %P \t %an \t %ae \t %aI \t %D \t %s \x1f %B
	gitShowMetaFormat = "%H%x09%P%x09%an%x09%ae%x09%aI%x09%D%x09%s%x1f%B"
)

// handleGitShow は GET /api/git-show を処理する。
// クエリ: session, token, hash
func (s *Server) handleGitShow(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	q := r.URL.Query()
	sid, ok := parseSessionID(q.Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	hash := strings.TrimSpace(q.Get("hash"))
	if hash == "" {
		writeGitError(w, http.StatusBadRequest, "bad_request", "hash is required")
		return
	}

	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	// 1) commit メタ取得
	metaOut, err := runGit(ctx, cwd, "show", "-s", "--decorate=short",
		"--pretty=format:"+gitShowMetaFormat, hash)
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
		return
	}
	meta, perr := parseGitShowMeta(string(metaOut))
	if perr != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", perr.Error())
		return
	}

	// 2) ファイル一覧 (status + path)
	nameOut, err := runGit(ctx, cwd, "show", "--name-status", "--pretty=format:", hash)
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
		return
	}
	files := parseNameStatus(string(nameOut))

	// 3) numstat で added/removed
	numOut, err := runGit(ctx, cwd, "show", "--numstat", "--pretty=format:", hash)
	if err == nil {
		applyNumstat(files, string(numOut))
	}

	// 4) ファイルごとの diff 取得（256KB 超は truncate）
	for i := range files {
		// 削除 (D) でも `git show <hash> -- <path>` は previous content を含む diff を返す
		diffOut, derr := runGit(ctx, cwd, "show", "--pretty=format:", hash, "--", files[i].Path)
		if derr != nil {
			// 個別エラーはスキップ（diff 空のまま）
			continue
		}
		diff := string(diffOut)
		// pretty=format: 直後は空行になることが多いので軽く trim
		diff = strings.TrimLeft(diff, "\n")
		if len(diff) > gitShowDiffMaxBytes {
			diff = diff[:gitShowDiffMaxBytes] + "\n(truncated)"
		}
		files[i].Diff = diff
	}

	resp := gitShowResp{
		OK:          true,
		Hash:        meta.hash,
		Parents:     meta.parents,
		AuthorName:  meta.authorName,
		AuthorEmail: meta.authorEmail,
		AuthorDate:  meta.authorDate,
		Subject:     meta.subject,
		Body:        meta.body,
		Refs:        meta.refs,
		Files:       files,
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

type gitShowMeta struct {
	hash        string
	parents     []string
	authorName  string
	authorEmail string
	authorDate  string
	subject     string
	body        string
	refs        []gitRef
}

// parseGitShowMeta は gitShowMetaFormat の出力をパースする。
// フォーマット: %H \t %P \t %an \t %ae \t %aI \t %D \t %s \x1f %B
func parseGitShowMeta(raw string) (gitShowMeta, error) {
	raw = strings.TrimRight(raw, "\n\r")
	// %B の最後に末尾改行が付くことがあるため、まず \x1f で subject / body 分離
	sep := strings.Index(raw, "\x1f")
	var headPart, body string
	if sep < 0 {
		// body 無し commit（subject = body 同一でも %B はあるはずだが念のため）
		headPart = raw
		body = ""
	} else {
		headPart = raw[:sep]
		body = strings.TrimSpace(raw[sep+1:])
	}
	// headPart は subject まで含む。subject はタブを含まないので 7 要素で SplitN。
	parts := strings.SplitN(headPart, "\t", 7)
	if len(parts) < 7 {
		return gitShowMeta{}, errMalformedGitShowMeta
	}
	parents := []string{}
	if pf := strings.TrimSpace(parts[1]); pf != "" {
		parents = strings.Fields(pf)
	}
	return gitShowMeta{
		hash:        parts[0],
		parents:     parents,
		authorName:  parts[2],
		authorEmail: parts[3],
		authorDate:  parts[4],
		refs:        parseDecorate(parts[5]),
		subject:     parts[6],
		body:        body,
	}, nil
}

var errMalformedGitShowMeta = malformedErr("malformed git show meta output")

type malformedErr string

func (e malformedErr) Error() string { return string(e) }

// parseNameStatus は `git show --name-status --pretty=format:` の出力をパースする。
// 行例:
//
//	A\tdocs/foo.md
//	M\tsrc/bar.go
//	R100\tsrc/old.go\tsrc/new.go    (rename の場合は new path を採用)
//	D\tobsolete.txt
func parseNameStatus(raw string) []gitShowFile {
	files := []gitShowFile{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		fields := strings.Split(line, "\t")
		if len(fields) < 2 {
			continue
		}
		status := fields[0]
		// 短縮: R100 / C75 等は先頭 1 文字で揃える
		shortStatus := status
		if len(status) > 0 {
			shortStatus = status[:1]
		}
		path := fields[1]
		// rename / copy は new path が末尾要素
		if (shortStatus == "R" || shortStatus == "C") && len(fields) >= 3 {
			path = fields[2]
		}
		files = append(files, gitShowFile{
			Status: shortStatus,
			Path:   path,
		})
	}
	return files
}

// applyNumstat は `git show --numstat --pretty=format:` の出力で
// files[i].Added / Removed を埋める。path 一致で照合。
//
// 行例:
//
//	120\t0\tdocs/foo.md
//	15\t8\tsrc/bar.go
//	-\t-\tbinary.png
//	0\t0\tsrc/old.go => src/new.go    (rename の場合)
func applyNumstat(files []gitShowFile, raw string) {
	// path → (added, removed) map
	type stat struct{ added, removed int }
	m := map[string]stat{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		fields := strings.SplitN(line, "\t", 3)
		if len(fields) < 3 {
			continue
		}
		added, _ := strconv.Atoi(fields[0]) // "-" は 0 として扱う（バイナリ）
		removed, _ := strconv.Atoi(fields[1])
		path := fields[2]
		// "old => new" / "{a => b}/x" 形式は new path 側を採用するため右辺優先
		if idx := strings.LastIndex(path, " => "); idx >= 0 {
			path = strings.TrimSpace(path[idx+4:])
			path = strings.TrimRight(path, "}")
			path = strings.TrimSpace(path)
		}
		m[path] = stat{added: added, removed: removed}
	}
	for i := range files {
		if v, ok := m[files[i].Path]; ok {
			files[i].Added = v.added
			files[i].Removed = v.removed
		}
	}
}

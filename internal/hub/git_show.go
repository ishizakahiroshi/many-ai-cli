package hub

import (
	"context"
	"net/http"
	"strconv"
	"strings"
)

// gitShowFile は /api/git-show の各ファイル diff エントリ。
type gitShowFile struct {
	Status  string `json:"status"` // A / M / D / R / C / T 等
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
	if !s.guard(w, r, http.MethodGet) {
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
	if !validRevision(hash) {
		writeGitError(w, http.StatusBadRequest, "bad_request", "invalid hash format")
		return
	}

	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	// 1) commit メタ取得（hash の後に -- を置いてファイルパスと明示的に区切る）
	metaOut, err := runGit(ctx, cwd, "show", "-s", "--decorate=short",
		"--pretty=format:"+gitShowMetaFormat, hash, "--")
	if err != nil {
		s.logger.Warn("git show metadata failed", "session_id", sid, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	meta, perr := parseGitShowMeta(string(metaOut))
	if perr != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", perr.Error())
		return
	}

	// 2) ファイル一覧 (status + path)
	nameOut, err := runGit(ctx, cwd, "show", "--name-status", "--pretty=format:", hash, "--")
	if err != nil {
		s.logger.Warn("git show names failed", "session_id", sid, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	files := parseNameStatus(string(nameOut))

	// 3) numstat で added/removed
	numOut, err := runGit(ctx, cwd, "show", "--numstat", "--pretty=format:", hash, "--")
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
	writeJSON(w, resp)
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

// numstatEntry は parseNumstatRaw の 1 件。
type numstatEntry struct{ added, removed int }

// parseNumstatRaw は `--numstat` 出力を path → (added, removed) マップにパースする。
// バイナリファイルの "-" は 0 として扱う。rename / copy の "{old => new}" 形式は
// new path 側を採用する。git_status の applyWorkingTreeNumstat と git_show の
// applyNumstat で共有する。
func parseNumstatRaw(raw string) map[string]numstatEntry {
	m := map[string]numstatEntry{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		fields := strings.SplitN(line, "\t", 3)
		if len(fields) < 3 {
			continue
		}
		added, _ := strconv.Atoi(fields[0])   // "-" は 0 として扱う（バイナリ）
		removed, _ := strconv.Atoi(fields[1]) //nolint:mnd
		path := gitNumstatRestoreRenamePath(strings.TrimRight(fields[2], "\r"))
		m[path] = numstatEntry{added: added, removed: removed}
	}
	return m
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
	m := parseNumstatRaw(raw)
	for i := range files {
		if v, ok := m[files[i].Path]; ok {
			files[i].Added = v.added
			files[i].Removed = v.removed
		}
	}
}

// gitNumstatRestoreRenamePath は git numstat の brace 圧縮リネーム表記から new path を復元する。
// git numstat は rename を "prefix/{old => new}/suffix" や "old => new" の形式で出力する。
//
// 例:
//
//	"old.go => new.go"           → "new.go"
//	"src/{old => new}/file.go"   → "src/new/file.go"
//	"src/{old => }/file.go"      → "src/file.go"
//	"internal/hub/git_show.go"   → "internal/hub/git_show.go"（変化なし）
func gitNumstatRestoreRenamePath(path string) string {
	// brace 圧縮形: "prefix/{old => new}/suffix"
	lBrace := strings.Index(path, "{")
	rBrace := strings.Index(path, "}")
	if lBrace >= 0 && rBrace > lBrace {
		inner := path[lBrace+1 : rBrace]
		arrowIdx := strings.Index(inner, " => ")
		if arrowIdx < 0 {
			// brace はあるが " => " がない → リネームではない
			return path
		}
		newSeg := strings.TrimSpace(inner[arrowIdx+4:])
		prefix := path[:lBrace]
		suffix := path[rBrace+1:]
		result := prefix + newSeg + suffix
		// 連続スラッシュを畳む（newSeg が空のとき "src//file.go" になる）
		for strings.Contains(result, "//") {
			result = strings.ReplaceAll(result, "//", "/")
		}
		result = strings.TrimSuffix(result, "/")
		return result
	}
	// 単純形: "old => new"
	if idx := strings.LastIndex(path, " => "); idx >= 0 {
		return strings.TrimSpace(path[idx+4:])
	}
	return path
}

// mapKeys は numstatEntry マップのキー一覧を返す（テスト用）。
func mapKeys(m map[string]numstatEntry) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	return keys
}

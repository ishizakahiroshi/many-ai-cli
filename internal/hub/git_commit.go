package hub

import (
	"context"
	"fmt"
	"net/http"
	"regexp"
	"strings"
)

const (
	gitCommitSubjectMaxLen = 200
	gitCommitBodyMaxLen    = 8192
	gitCommitDiffMaxBytes  = 48 * 1024
)

type gitCommitAllReq struct {
	Session int    `json:"session"`
	Token   string `json:"token"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
}

type gitCommitAllResp struct {
	OK           bool   `json:"ok"`
	Hash         string `json:"hash"`
	ShortHash    string `json:"short_hash"`
	Subject      string `json:"subject"`
	FilesChanged int    `json:"files_changed"`
}

type gitCommitMessageReq struct {
	Session  int    `json:"session"`
	Token    string `json:"token"`
	Mode     string `json:"mode"`
	Language string `json:"language"`
}

type gitCommitMessageResp struct {
	OK      bool   `json:"ok"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
	// Pending は mode=="ai" のとき true。結果は別途 WS（commit_msg_suggested）で届く。
	Pending bool `json:"pending,omitempty"`
}

func (s *Server) handleGitCommitAll(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req gitCommitAllReq
	if !decodeJSON(w, r, &req) {
		return
	}
	subject := sanitizeCommitMessage(req.Subject, gitCommitSubjectMaxLen)
	body := sanitizeCommitMessage(req.Body, gitCommitBodyMaxLen)
	if subject == "" {
		writeGitError(w, http.StatusBadRequest, "bad_request", "subject is required")
		return
	}
	if req.Session <= 0 {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, cwd, err := s.resolveGitRoot(req.Session)
	if err != nil {
		writeGitErrorFromResolve(w, req.Session, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	statusOut, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1", "-z")
	if err != nil {
		s.logger.Warn("git status failed before commit", "session_id", req.Session, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	filesChanged := len(parseGitStatusPorcelainZ(string(statusOut)))
	if filesChanged == 0 {
		writeGitError(w, http.StatusBadRequest, "no_changes", "working tree has no changes")
		return
	}
	if _, err := runGitCombined(ctx, gitRoot, "add", "-A"); err != nil {
		code, status := classifyGitCommitError(err)
		s.logger.Warn("git add failed before commit", "session_id", req.Session, "err", err)
		writeGitError(w, status, code, sanitizeGitErrMsg(err))
		return
	}
	if _, err := runGit(ctx, gitRoot, "diff", "--cached", "--quiet"); err == nil {
		writeGitError(w, http.StatusBadRequest, "no_changes", "no staged changes after git add -A")
		return
	}
	args := []string{"commit", "-m", subject}
	if body != "" {
		args = append(args, "-m", body)
	}
	if _, err := runGitCombined(ctx, gitRoot, args...); err != nil {
		code, status := classifyGitCommitError(err)
		s.logger.Warn("git commit failed", "session_id", req.Session, "err", err)
		writeGitError(w, status, code, sanitizeGitErrMsg(err))
		return
	}
	hash := ""
	if out, err := runGit(ctx, gitRoot, "rev-parse", "HEAD"); err == nil {
		hash = strings.TrimSpace(string(out))
	}
	shortHash := ""
	if out, err := runGit(ctx, gitRoot, "rev-parse", "--short", "HEAD"); err == nil {
		shortHash = strings.TrimSpace(string(out))
	}
	writeJSON(w, gitCommitAllResp{
		OK:           true,
		Hash:         hash,
		ShortHash:    shortHash,
		Subject:      subject,
		FilesChanged: filesChanged,
	})
}

func (s *Server) handleGitCommitMessage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req gitCommitMessageReq
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.Session <= 0 {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	_, cwd, err := s.resolveGitRoot(req.Session)
	if err != nil {
		writeGitErrorFromResolve(w, req.Session, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	statusOut, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1", "-z")
	if err != nil {
		s.logger.Warn("git status failed before commit message", "session_id", req.Session, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	files := parseGitStatusPorcelainZ(string(statusOut))
	if len(files) == 0 {
		writeGitError(w, http.StatusBadRequest, "no_changes", "working tree has no changes")
		return
	}
	// mode=="ai": 接続中の AI セッションへ生成プロンプトを注入し、PTY 出力から
	// マーカーを拾ってフォームへ反映する。結果は WS（commit_msg_suggested）で届くため、
	// ここでは pending を返して即応答する。
	if strings.EqualFold(req.Mode, "ai") {
		s.startAICommitMessage(w, req.Session, req.Language)
		return
	}
	stat := ""
	if out, err := runGit(ctx, cwd, "diff", "--stat", "HEAD", "--"); err == nil {
		stat = strings.TrimSpace(string(out))
	}
	diff := ""
	diffNotice := ""
	if out, err := runGit(ctx, cwd, "diff", "HEAD", "--"); err == nil {
		diff = string(out)
		if len(diff) > gitCommitDiffMaxBytes {
			diff = diff[:gitCommitDiffMaxBytes]
			diffNotice = fmt.Sprintf("Diff context truncated to %d KiB.", gitCommitDiffMaxBytes/1024)
		}
	}

	subject, body := suggestCommitMessage(files, stat, diff, diffNotice, req.Language)
	writeJSON(w, gitCommitMessageResp{
		OK:      true,
		Subject: subject,
		Body:    body,
	})
}

// commitChangeAnalysis は working tree の差分から導いた、コミットメッセージ生成用の
// 解析結果。LLM を使わず status とテキスト差分のヒューリスティックで埋めるため、
// あくまで「下書き」レベルの精度であることを前提とする。
type commitChangeAnalysis struct {
	added       []string // 新規追加されたファイルパス（A / 未追跡 ??）
	deleted     []string // 削除されたファイルパス（D）
	modified    []string // 変更されたファイルパス（M ほか）
	renamed     []string // リネームされたファイルパス（R）
	depFiles    []string // 依存定義ファイル（go.mod / package.json 等）
	topScope    string   // 最も変更ファイル数が多いトップレベルディレクトリ
	prefix      string   // conventional commit prefix（feat/fix/docs/test/style/refactor/chore）
	routes      []string // 追加された HTTP ルート（mux.HandleFunc("...")）
	removedRts  []string // 削除された HTTP ルート
	funcs       []string // 追加された Go 関数名
	types       []string // 追加された Go 型（struct / interface）
	renamePairs []string // "旧名 → 新名"（diff の rename from/to から）
	i18nKeys    int      // 追加された i18n キー数
	depsOnly    bool     // 変更が依存定義ファイルのみ
	styleOnly   bool     // 変更が CSS/SCSS のみ
}

var (
	reGitRoute    = regexp.MustCompile(`mux\.HandleFunc\("([^"]+)"`)
	reAddedGoFunc = regexp.MustCompile(`^\+func (?:\([^)]*\)\s*)?([A-Za-z0-9_]+)\(`)
	reAddedGoType = regexp.MustCompile(`^\+type ([A-Za-z0-9_]+) (?:struct|interface)\b`)
	reAddedI18n   = regexp.MustCompile(`^\+\s*"[A-Za-z0-9_]+":`)
	reDiffNewFile = regexp.MustCompile(`^\+\+\+ b/(.+)$`)
)

var depFileNames = map[string]struct{}{
	"go.mod": {}, "go.sum": {}, "go.work": {}, "go.work.sum": {},
	"package.json": {}, "package-lock.json": {}, "bun.lockb": {},
	"yarn.lock": {}, "pnpm-lock.yaml": {},
}

func suggestCommitMessage(files []gitStatusFile, stat, diff, diffNotice, language string) (string, string) {
	ja := strings.EqualFold(language, "ja") || language == ""
	a := analyzeCommitChanges(files, diff)
	subject := a.subjectLine(ja)
	body := a.bodyText(ja, stat, diffNotice, len(files))
	return sanitizeCommitMessage(subject, gitCommitSubjectMaxLen), sanitizeCommitMessage(body, gitCommitBodyMaxLen)
}

func analyzeCommitChanges(files []gitStatusFile, diff string) commitChangeAnalysis {
	a := commitChangeAnalysis{}
	scopeCounts := map[string]int{}
	scopeBestN := 0
	docOnly, testOnly, depsOnly, styleOnly, codeChange, hasFile := true, true, true, true, false, false
	for _, f := range files {
		p := strings.ReplaceAll(f.Path, "\\", "/")
		if p == "" {
			continue
		}
		hasFile = true
		switch f.Status {
		case "A", "??":
			a.added = append(a.added, p)
		case "D":
			a.deleted = append(a.deleted, p)
		case "R":
			a.renamed = append(a.renamed, p)
		default:
			a.modified = append(a.modified, p)
		}
		if _, ok := depFileNames[baseName(p)]; ok {
			a.depFiles = append(a.depFiles, p)
		} else {
			depsOnly = false
		}
		if !isDocPath(p) {
			docOnly = false
		}
		if !isTestPath(p) {
			testOnly = false
		}
		if !isStylePath(p) {
			styleOnly = false
		}
		if strings.HasPrefix(p, "web/") || strings.HasPrefix(p, "internal/") || strings.HasPrefix(p, "cmd/") || strings.HasPrefix(p, "pkg/") {
			codeChange = true
		}
		scope := p
		if idx := strings.Index(p, "/"); idx > 0 {
			scope = p[:idx]
		}
		scopeCounts[scope]++
		if scopeCounts[scope] > scopeBestN {
			a.topScope = scope
			scopeBestN = scopeCounts[scope]
		}
	}
	a.depsOnly = hasFile && depsOnly
	a.styleOnly = hasFile && styleOnly

	a.scanDiff(diff)

	// リネームのみ・追加削除を伴わない編集は refactor 寄りに分類する。
	renameOnly := len(a.renamed) > 0 && len(a.added) == 0 && len(a.deleted) == 0 && len(a.modified) == 0
	// 具体的な新規シンボル（新規ファイル・関数・型・HTTP ルート）が検出できた場合のみ
	// feat 扱いにする。これが無い「既存コードの編集だけ」を一律 feat にしないことで、
	// 公開履歴が feat: で埋まるのを防ぐ（方針1: prefix 推定の正確化）。
	hasAdditions := len(a.added) > 0 || len(a.funcs) > 0 || len(a.types) > 0 || len(a.routes) > 0
	hasRemovals := len(a.deleted) > 0 || len(a.removedRts) > 0
	switch {
	case !hasFile:
		a.prefix = "chore"
	case a.depsOnly:
		a.prefix = "chore(deps)"
	case docOnly:
		a.prefix = "docs"
	case testOnly:
		a.prefix = "test"
	case a.styleOnly:
		a.prefix = "style"
	case renameOnly:
		a.prefix = "refactor"
	case hasAdditions:
		a.prefix = "feat"
	case codeChange && (hasRemovals || len(a.modified) > 0):
		// 新規シンボルを伴わないコード変更（削除・既存編集のみ）は refactor 寄り。
		a.prefix = "refactor"
	case codeChange:
		a.prefix = "feat"
	default:
		a.prefix = "chore"
	}
	return a
}

// scanDiff は差分テキストから「追加/削除された HTTP ルート・追加された Go 関数/型・
// 追加された i18n キー・リネーム対」を拾う。+++ b/<path> ヘッダで処理中ファイルを追跡し、
// rename from/to メタ行でリネーム対を組む。あくまで正規表現ベースの近似抽出。
func (a *commitChangeAnalysis) scanDiff(diff string) {
	if diff == "" {
		return
	}
	seenRoute := map[string]struct{}{}
	seenRmRoute := map[string]struct{}{}
	seenFunc := map[string]struct{}{}
	seenType := map[string]struct{}{}
	cur := ""
	renameOld := ""
	for _, line := range strings.Split(diff, "\n") {
		if m := reDiffNewFile.FindStringSubmatch(line); m != nil {
			cur = m[1]
			continue
		}
		if strings.HasPrefix(line, "rename from ") {
			renameOld = strings.TrimSpace(strings.TrimPrefix(line, "rename from "))
			continue
		}
		if strings.HasPrefix(line, "rename to ") {
			newName := strings.TrimSpace(strings.TrimPrefix(line, "rename to "))
			if renameOld != "" {
				a.renamePairs = append(a.renamePairs, baseName(renameOld)+" → "+baseName(newName))
				renameOld = ""
			}
			continue
		}
		isAdd := strings.HasPrefix(line, "+") && !strings.HasPrefix(line, "++")
		isDel := strings.HasPrefix(line, "-") && !strings.HasPrefix(line, "--")
		switch {
		case isAdd:
			if m := reGitRoute.FindStringSubmatch(line); m != nil {
				addUnique(&a.routes, seenRoute, m[1])
			}
			if strings.HasSuffix(cur, ".go") {
				if m := reAddedGoFunc.FindStringSubmatch(line); m != nil {
					addUnique(&a.funcs, seenFunc, m[1])
				}
				if m := reAddedGoType.FindStringSubmatch(line); m != nil {
					addUnique(&a.types, seenType, m[1])
				}
			}
			if strings.HasSuffix(cur, ".json") && strings.Contains(cur, "i18n") && reAddedI18n.MatchString(line) {
				a.i18nKeys++
			}
		case isDel:
			if m := reGitRoute.FindStringSubmatch(line); m != nil {
				addUnique(&a.removedRts, seenRmRoute, m[1])
			}
		}
	}
	// 追加・削除の両方に現れたルートは「変更（行移動）」であって新規/削除ではないため、
	// 両側から取り除く。
	a.routes = filterOutSet(a.routes, seenRmRoute)
	a.removedRts = filterOutSet(a.removedRts, seenRoute)
}

func addUnique(dst *[]string, seen map[string]struct{}, v string) {
	if _, ok := seen[v]; ok {
		return
	}
	seen[v] = struct{}{}
	*dst = append(*dst, v)
}

// filterOutSet は items から set に含まれる要素を除いた新しいスライスを返す。
func filterOutSet(items []string, set map[string]struct{}) []string {
	if len(items) == 0 || len(set) == 0 {
		return items
	}
	out := items[:0:0]
	for _, it := range items {
		if _, ok := set[it]; !ok {
			out = append(out, it)
		}
	}
	return out
}

func (a commitChangeAnalysis) subjectLine(ja bool) string {
	scope := a.topScope
	if scope == "" {
		scope = "working tree"
	}
	renameOnly := len(a.renamePairs) > 0 && len(a.added) == 0 && len(a.deleted) == 0 && len(a.modified) == 0
	if ja {
		var what string
		switch {
		case a.depsOnly:
			what = "依存関係を更新"
		case a.styleOnly:
			what = "スタイルを調整"
		case renameOnly:
			what = withMoreJa(a.renamePairs[0], len(a.renamePairs)) + " にリネーム"
		case len(a.routes) > 0:
			what = withMoreJa(a.routes[0], len(a.routes)) + " エンドポイントを追加"
		case len(a.added) > 0:
			what = withMoreJa(baseName(a.added[0]), len(a.added)) + " を追加"
		case len(a.funcs) > 0:
			what = withMoreJa(a.funcs[0], len(a.funcs)) + " を追加"
		case len(a.types) > 0:
			what = withMoreJa(a.types[0], len(a.types)) + " 型を追加"
		case len(a.removedRts) > 0:
			what = withMoreJa(a.removedRts[0], len(a.removedRts)) + " エンドポイントを削除"
		case len(a.deleted) > 0 && len(a.modified) == 0:
			what = withMoreJa(baseName(a.deleted[0]), len(a.deleted)) + " を削除"
		case len(a.modified) > 0:
			// 無情報な「scope を更新」を避け、代表的な変更ファイル名で要約する（方針2）。
			what = withMoreJa(baseName(a.modified[0]), len(a.modified)) + " を変更"
		default:
			what = scope + " を更新"
		}
		return a.prefix + ": " + what
	}
	var what string
	switch {
	case a.depsOnly:
		what = "update dependencies"
	case a.styleOnly:
		what = "tweak styles"
	case renameOnly:
		what = "rename " + withMoreEn(a.renamePairs[0], len(a.renamePairs))
	case len(a.routes) > 0:
		what = "add " + withMoreEn(a.routes[0], len(a.routes)) + " endpoint(s)"
	case len(a.added) > 0:
		what = "add " + withMoreEn(baseName(a.added[0]), len(a.added))
	case len(a.funcs) > 0:
		what = "add " + withMoreEn(a.funcs[0], len(a.funcs))
	case len(a.types) > 0:
		what = "add " + withMoreEn(a.types[0], len(a.types)) + " type(s)"
	case len(a.removedRts) > 0:
		what = "remove " + withMoreEn(a.removedRts[0], len(a.removedRts)) + " endpoint(s)"
	case len(a.deleted) > 0 && len(a.modified) == 0:
		what = "remove " + withMoreEn(baseName(a.deleted[0]), len(a.deleted))
	case len(a.modified) > 0:
		what = "update " + withMoreEn(baseName(a.modified[0]), len(a.modified))
	default:
		what = "update " + scope
	}
	return a.prefix + ": " + what
}

func (a commitChangeAnalysis) bodyText(ja bool, stat, diffNotice string, total int) string {
	var lines []string
	if ja {
		counts := fmt.Sprintf("ファイル %d 件（新規 %d / 変更 %d / 削除 %d", total, len(a.added), len(a.modified), len(a.deleted))
		if len(a.renamed) > 0 {
			counts += fmt.Sprintf(" / 改名 %d", len(a.renamed))
		}
		counts += "）。"
		lines = append(lines, counts)
		if len(a.added) > 0 {
			lines = append(lines, "- 新規: "+joinBaseNames(a.added, 5, ja))
		}
		if len(a.deleted) > 0 {
			lines = append(lines, "- 削除: "+joinBaseNames(a.deleted, 5, ja))
		}
		if len(a.renamePairs) > 0 {
			lines = append(lines, "- 改名: "+joinList(a.renamePairs, 5, ja))
		}
		if len(a.depFiles) > 0 {
			lines = append(lines, "- 依存: "+joinBaseNames(a.depFiles, 5, ja)+" を更新")
		}
		if len(a.routes) > 0 {
			lines = append(lines, "- API追加: "+joinList(a.routes, 6, ja))
		}
		if len(a.removedRts) > 0 {
			lines = append(lines, "- API削除: "+joinList(a.removedRts, 6, ja))
		}
		if a.i18nKeys > 0 {
			lines = append(lines, fmt.Sprintf("- i18n: %d 件のキーを追加", a.i18nKeys))
		}
		if len(a.types) > 0 {
			lines = append(lines, "- 型: "+joinList(a.types, 6, ja)+" を追加")
		}
		if len(a.routes) == 0 && len(a.funcs) > 0 {
			lines = append(lines, "- 関数: "+joinList(a.funcs, 6, ja)+" を追加")
		}
	} else {
		counts := fmt.Sprintf("%d file(s): %d added / %d modified / %d deleted", total, len(a.added), len(a.modified), len(a.deleted))
		if len(a.renamed) > 0 {
			counts += fmt.Sprintf(" / %d renamed", len(a.renamed))
		}
		counts += "."
		lines = append(lines, counts)
		if len(a.added) > 0 {
			lines = append(lines, "- New: "+joinBaseNames(a.added, 5, ja))
		}
		if len(a.deleted) > 0 {
			lines = append(lines, "- Removed: "+joinBaseNames(a.deleted, 5, ja))
		}
		if len(a.renamePairs) > 0 {
			lines = append(lines, "- Renamed: "+joinList(a.renamePairs, 5, ja))
		}
		if len(a.depFiles) > 0 {
			lines = append(lines, "- Deps: updated "+joinBaseNames(a.depFiles, 5, ja))
		}
		if len(a.routes) > 0 {
			lines = append(lines, "- API added: "+joinList(a.routes, 6, ja))
		}
		if len(a.removedRts) > 0 {
			lines = append(lines, "- API removed: "+joinList(a.removedRts, 6, ja))
		}
		if a.i18nKeys > 0 {
			lines = append(lines, fmt.Sprintf("- i18n: added %d key(s)", a.i18nKeys))
		}
		if len(a.types) > 0 {
			lines = append(lines, "- Types: added "+joinList(a.types, 6, ja))
		}
		if len(a.routes) == 0 && len(a.funcs) > 0 {
			lines = append(lines, "- Functions: added "+joinList(a.funcs, 6, ja))
		}
	}
	body := strings.Join(lines, "\n")
	if stat != "" {
		body += "\n\n" + stat
	}
	if diffNotice != "" {
		body += "\n\n" + diffNotice
	}
	return body
}

func isDocPath(p string) bool {
	if strings.HasPrefix(p, "docs/") || strings.HasPrefix(p, "README") || strings.HasPrefix(p, "CHANGELOG") {
		return true
	}
	return strings.HasSuffix(p, ".md")
}

func isTestPath(p string) bool {
	return strings.HasSuffix(p, "_test.go") || strings.Contains(p, ".test.") || strings.Contains(p, ".spec.")
}

func isStylePath(p string) bool {
	return strings.HasSuffix(p, ".css") || strings.HasSuffix(p, ".scss")
}

func baseName(p string) string {
	p = strings.ReplaceAll(p, "\\", "/")
	if idx := strings.LastIndex(p, "/"); idx >= 0 {
		return p[idx+1:]
	}
	return p
}

// withMoreJa は先頭要素に「ほか N 件」を付ける（total<=1 なら head のみ）。
func withMoreJa(head string, total int) string {
	if total > 1 {
		return fmt.Sprintf("%s ほか %d 件", head, total-1)
	}
	return head
}

func withMoreEn(head string, total int) string {
	if total > 1 {
		return fmt.Sprintf("%s (+%d more)", head, total-1)
	}
	return head
}

// joinBaseNames はパス集合を basename で max 件まで連結し、超過分は丸める。
func joinBaseNames(paths []string, max int, ja bool) string {
	bases := make([]string, 0, len(paths))
	for _, p := range paths {
		bases = append(bases, baseName(p))
	}
	return joinList(bases, max, ja)
}

func joinList(items []string, max int, ja bool) string {
	if len(items) <= max {
		return strings.Join(items, ", ")
	}
	more := len(items) - max
	if ja {
		return strings.Join(items[:max], ", ") + fmt.Sprintf(" ほか %d 件", more)
	}
	return strings.Join(items[:max], ", ") + fmt.Sprintf(" (+%d more)", more)
}

package hub

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
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
	weights := map[string]int{}
	if out, err := runGit(ctx, cwd, "diff", "--numstat", "--no-renames", "HEAD", "--"); err == nil {
		weights = parseGitNumstat(string(out))
	}
	for _, file := range files {
		if file.Status != "??" {
			continue
		}
		path := strings.ReplaceAll(file.Path, "\\", "/")
		if _, exists := weights[path]; !exists {
			weights[path] = countUntrackedFileLines(cwd, path)
		}
	}

	subject, body := suggestCommitMessageWithWeights(files, stat, diff, diffNotice, req.Language, weights)
	writeJSON(w, gitCommitMessageResp{
		OK:      true,
		Subject: subject,
		Body:    body,
	})
}

func parseGitNumstat(raw string) map[string]int {
	weights := map[string]int{}
	for _, line := range strings.Split(raw, "\n") {
		parts := strings.SplitN(strings.TrimSuffix(line, "\r"), "\t", 3)
		if len(parts) != 3 {
			continue
		}
		path := strings.ReplaceAll(parts[2], "\\", "/")
		if path == "" {
			continue
		}
		added := parseNumstatCount(parts[0])
		deleted := parseNumstatCount(parts[1])
		weights[path] = added + deleted
	}
	return weights
}

func parseNumstatCount(value string) int {
	if value == "-" {
		return 0
	}
	n := 0
	for _, r := range value {
		if r < '0' || r > '9' {
			return 0
		}
		n = n*10 + int(r-'0')
	}
	return n
}

func countUntrackedFileLines(root, path string) int {
	full := filepath.Join(root, filepath.FromSlash(path))
	rel, err := filepath.Rel(root, full)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return 0
	}
	f, err := os.Open(full)
	if err != nil {
		return 0
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, 1<<20))
	if err != nil || bytes.IndexByte(data, 0) >= 0 || len(data) == 0 {
		return 0
	}
	lines := bytes.Count(data, []byte{'\n'})
	if data[len(data)-1] != '\n' {
		lines++
	}
	return lines
}

// commitChangeAnalysis は working tree の差分から導いた、コミットメッセージ生成用の
// 解析結果。LLM を使わず status とテキスト差分のヒューリスティックで埋めるため、
// あくまで「下書き」レベルの精度であることを前提とする。
type commitChangeAnalysis struct {
	added            []string          // 新規追加されたファイルパス（A / 未追跡 ??）
	deleted          []string          // 削除されたファイルパス（D）
	modified         []string          // 変更されたファイルパス（M ほか）
	renamed          []string          // リネームされたファイルパス（R）
	depFiles         []string          // 依存定義ファイル（go.mod / package.json 等）
	scope            string            // 変更ファイル群の最深共通ディレクトリの最終非汎用セグメント（Conventional Commits の (scope) 部分）
	prefix           string            // conventional commit prefix（feat/fix/docs/test/style/refactor/chore）
	routes           []string          // 追加された HTTP ルート（mux.HandleFunc("...")）
	removedRts       []string          // 削除された HTTP ルート
	funcs            []string          // 追加された関数名（Go / TS / JS / Python、出現順）
	deletedFuncs     []string          // 削除された Go 関数名（move 判定用・出現順）
	addedFuncSites   map[string]string // 関数名 → 追加された diff の +++ b/<file>
	deletedFuncSites map[string]string // 関数名 → 削除された diff の +++ b/<file>
	types            []string          // 追加された型・クラス（Go / TS / JS / Python）
	addedTypeSites   map[string]string // 型名 → 追加された diff の +++ b/<file>
	addedRouteSites  map[string]string // ルート → 追加された diff の +++ b/<file>
	renamePairs      []string          // "旧名 → 新名"（diff の rename from/to から）
	i18nKeys         int               // 追加された i18n キー数
	locAdded         int               // 追加行数（+++ ヘッダを除く +行）
	locDeleted       int               // 削除行数（--- ヘッダを除く -行）
	handleAdds       int               // 追加された err/throw/catch など handling パターン行数
	depsOnly         bool              // 変更が依存定義ファイルのみ
	styleOnly        bool              // 変更が CSS/SCSS のみ
	verb             string            // subject 用に選ばれた動詞
	symbolHead       string            // subject の主辞（関数名 / 型名 / route / ファイル basename 等）
	symbolCount      int               // symbolHead を含めた同種要素の総件数
	weights          map[string]int
	dominant         string
	dominantStatus   string
}

var (
	reGoRoute       = regexp.MustCompile(`mux\.HandleFunc\("([^"]+)"`)
	reJSRoute       = regexp.MustCompile(`\b(?:app|router)\.(?:get|post|put|delete|patch)\(\s*['"]([^'"]+)`)
	rePythonRoute   = regexp.MustCompile(`^[+-]@(?:app|router|bp)\.(?:get|post|put|delete|patch|route)\(\s*['"]([^'"]+)`)
	reAddedGoFunc   = regexp.MustCompile(`^\+func (?:\([^)]*\)\s*)?([A-Za-z0-9_]+)\(`)
	reDeletedGoFunc = regexp.MustCompile(`^-func (?:\([^)]*\)\s*)?([A-Za-z0-9_]+)\(`)
	reAddedGoType   = regexp.MustCompile(`^\+type ([A-Za-z0-9_]+) (?:struct|interface)\b`)
	reAddedJSFunc   = regexp.MustCompile(`^\+export (?:default )?(?:async )?function ([A-Za-z0-9_$]+)`)
	reAddedJSConst  = regexp.MustCompile(`^\+export const ([A-Za-z0-9_$]+)\s*=`)
	reAddedJSClass  = regexp.MustCompile(`^\+export (?:default )?(?:abstract )?class ([A-Za-z0-9_$]+)`)
	reAddedJSType   = regexp.MustCompile(`^\+export (?:type|interface) ([A-Za-z0-9_$]+)`)
	reAddedPyFunc   = regexp.MustCompile(`^\+def ([A-Za-z0-9_]+)\(`)
	reAddedPyClass  = regexp.MustCompile(`^\+class ([A-Za-z0-9_]+)`)
	reAddedI18n     = regexp.MustCompile(`^\+\s*['"]?[A-Za-z0-9_.-]+['"]?\s*:`)
	reDiffNewFile   = regexp.MustCompile(`^\+\+\+ b/(.+)$`)
	reHandleErrGo   = regexp.MustCompile(`^\+\s*if\s+err\s*!=`)
	reHandleThrow   = regexp.MustCompile(`^\+\s*throw\s`)
	reHandleCatch   = regexp.MustCompile(`^\+\s*(?:\}\s*)?catch\b`)
)

// scope 決定時に「情報量の薄い汎用セグメント」として除外する dir 名。
// deepestScope はここに該当しない最も深いセグメントを返す（例: web/src/app → app）。
var genericScopeSegments = map[string]bool{
	"src": true, "internal": true, "cmd": true, "pkg": true, "lib": true,
}

// deepestScope は変更ファイル群の最深共通ディレクトリを求め、その末尾から見て
// 最初の非汎用セグメント（internal/src 等でないもの）を返す。全ファイルが同一
// トップレベル配下でない場合は空文字を返す（＝ scope なし＝横断変更の合図）。
func deepestScope(paths []string) string {
	if len(paths) == 0 {
		return ""
	}
	var common []string
	for i, p := range paths {
		p = strings.ReplaceAll(p, "\\", "/")
		parts := strings.Split(p, "/")
		if len(parts) > 0 {
			parts = parts[:len(parts)-1]
		}
		if i == 0 {
			common = append([]string(nil), parts...)
			continue
		}
		n := 0
		for n < len(common) && n < len(parts) && common[n] == parts[n] {
			n++
		}
		common = common[:n]
		if len(common) == 0 {
			return ""
		}
	}
	for i := len(common) - 1; i >= 0; i-- {
		if !genericScopeSegments[common[i]] {
			return common[i]
		}
	}
	return ""
}

// prefixWithScope は Conventional Commits の "<type>(<scope>)" を組み立てる。
// scope が空、prefix と一致、または prefix が既にカッコ付き（chore(deps) 等）の場合は
// scope を付けない。
func prefixWithScope(prefix, scope string) string {
	if scope == "" || scope == prefix {
		return prefix
	}
	if strings.Contains(prefix, "(") {
		return prefix
	}
	return prefix + "(" + scope + ")"
}

// verbJa / verbEn は 9 種＋update の動詞 → 各言語の語のマッピング。
// jaVerbPhrase / enVerbPhrase 経由で subject に埋められる。
var verbJa = map[string]string{
	"add":      "追加",
	"remove":   "削除",
	"rename":   "改名",
	"move":     "移動",
	"bump":     "更新",
	"simplify": "簡潔化",
	"handle":   "エラー処理を追加",
	"refactor": "整理",
	"test":     "テストを追加",
	"update":   "更新",
	"change":   "変更",
}

var verbEn = map[string]string{
	"add":      "add",
	"remove":   "remove",
	"rename":   "rename",
	"move":     "move",
	"bump":     "bump",
	"simplify": "simplify",
	"handle":   "handle errors in",
	"refactor": "rework",
	"test":     "add tests for",
	"update":   "update",
	"change":   "change",
}

// jaVerbPhrase は「<symbol>」の直後に付ける日本語述語を返す。助詞が「を」以外に
// なる動詞（改名の「に」、handle の「に」）は個別ケースで扱う。
func jaVerbPhrase(verb, head string) string {
	switch verb {
	case "handle":
		return head + " にエラー処理を追加"
	case "rename":
		return head + " に改名"
	default:
		v := verbJa[verb]
		if v == "" {
			v = verbJa["refactor"]
		}
		return head + " を" + v
	}
}

// enVerbPhrase は英語 subject の "<verb> <symbol>" 句を返す。handle だけは
// "handle errors in <symbol>" と展開する。
func enVerbPhrase(verb, head string) string {
	switch verb {
	case "handle":
		return "handle errors in " + head
	default:
		v := verbEn[verb]
		if v == "" {
			v = verbEn["refactor"]
		}
		return v + " " + head
	}
}

var depFileNames = map[string]struct{}{
	"go.mod": {}, "go.sum": {}, "go.work": {}, "go.work.sum": {},
	"package.json": {}, "package-lock.json": {}, "bun.lockb": {},
	"yarn.lock": {}, "pnpm-lock.yaml": {},
}

func suggestCommitMessage(files []gitStatusFile, stat, diff, diffNotice, language string) (string, string) {
	return suggestCommitMessageWithWeights(files, stat, diff, diffNotice, language, nil)
}

func suggestCommitMessageWithWeights(files []gitStatusFile, stat, diff, diffNotice, language string, weights map[string]int) (string, string) {
	ja := strings.EqualFold(language, "ja") || language == ""
	a := analyzeCommitChangesWithWeights(files, diff, weights)
	subject := a.subjectLine(ja)
	body := a.bodyText(ja, stat, diffNotice, len(files))
	return sanitizeCommitMessage(subject, gitCommitSubjectMaxLen), sanitizeCommitMessage(body, gitCommitBodyMaxLen)
}

func analyzeCommitChanges(files []gitStatusFile, diff string) commitChangeAnalysis {
	return analyzeCommitChangesWithWeights(files, diff, nil)
}

func analyzeCommitChangesWithWeights(files []gitStatusFile, diff string, weights map[string]int) commitChangeAnalysis {
	a := commitChangeAnalysis{weights: weights}
	docOnly, testOnly, depsOnly, styleOnly, codeChange, hasFile := true, true, true, true, false, false
	allPaths := make([]string, 0, len(files))
	for _, f := range files {
		p := strings.ReplaceAll(f.Path, "\\", "/")
		if p == "" {
			continue
		}
		hasFile = true
		allPaths = append(allPaths, p)
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
	a.scope = deepestScope(allPaths)
	a.selectDominant(files)
	a.determineVerbAndSymbol()
	return a
}

func (a *commitChangeAnalysis) selectDominant(files []gitStatusFile) {
	bestWeight := -1
	bestRank := -1
	for _, file := range files {
		path := strings.ReplaceAll(file.Path, "\\", "/")
		if path == "" {
			continue
		}
		weight := a.weights[path]
		rank := dominantStatusRank(file.Status)
		if weight > bestWeight ||
			(weight == bestWeight && rank > bestRank) ||
			(weight == bestWeight && rank == bestRank && (a.dominant == "" || path < a.dominant)) {
			a.dominant = path
			a.dominantStatus = file.Status
			bestWeight = weight
			bestRank = rank
		}
	}
}

func dominantStatusRank(status string) int {
	switch status {
	case "A", "??":
		return 3
	case "D":
		return 1
	default:
		return 2
	}
}

// determineVerbAndSymbol は analyze の最後で呼ばれ、subject に載せる動詞と主辞を
// 一意に決める。優先順は「情報量が多い動詞から」で、最後まで決まらなければ
// prefix の系統（docs/style/chore は update、それ以外は refactor）にフォールバック。
func (a *commitChangeAnalysis) determineVerbAndSymbol() {
	hasAdditions := len(a.added) > 0 || len(a.funcs) > 0 || len(a.types) > 0 || len(a.routes) > 0
	hasRemovals := len(a.deleted) > 0 || len(a.removedRts) > 0
	renameOnly := len(a.renamePairs) > 0 && !hasAdditions && !hasRemovals && len(a.modified) == 0
	// move: 同じ関数名が「別ファイル」で削除＋追加されている場合に真の "move" とみなす。
	// 同一ファイル内の削除＋追加（＝関数本体の書き換え）は refactor 扱いで、move には
	// 昇格させない（誤検出防止・addedFuncSites / deletedFuncSites でファイルを見る）。
	var moved []string
	for _, name := range a.funcs {
		addFile, hasAdd := a.addedFuncSites[name]
		delFile, hasDel := a.deletedFuncSites[name]
		if hasAdd && hasDel && addFile != delFile {
			moved = append(moved, name)
		}
	}
	dominantAddHead, dominantAddCount := a.pickAddedSymbolForFile(a.dominant)
	switch {
	case a.depsOnly:
		a.verb = "bump"
		if len(a.depFiles) > 0 {
			a.symbolHead = baseName(a.depFiles[0])
			a.symbolCount = len(a.depFiles)
		}
	case renameOnly:
		a.verb = "rename"
		a.symbolHead = a.renamePairs[0]
		a.symbolCount = len(a.renamePairs)
	case len(moved) > 0:
		a.verb = "move"
		a.symbolHead = moved[0]
		a.symbolCount = len(moved)
	case isModifiedStatus(a.dominantStatus) && len(a.added) > 0 &&
		dominantAddHead == "":
		a.verb = "change"
		a.symbolHead = baseName(a.dominant)
		a.symbolCount = len(a.modified)
	case isModifiedStatus(a.dominantStatus) && len(a.added) > 0 &&
		dominantAddHead != "" && !hasRemovals:
		a.verb = "add"
		a.symbolHead = dominantAddHead
		a.symbolCount = dominantAddCount
	case hasAdditions && !hasRemovals:
		a.verb = "add"
		a.symbolHead, a.symbolCount = pickAddSymbol(a)
	case hasRemovals && !hasAdditions:
		a.verb = "remove"
		a.symbolHead, a.symbolCount = pickRemoveSymbol(a)
	case a.handleAdds >= 3 && a.handleAdds*2 >= a.locAdded:
		a.verb = "handle"
		a.symbolHead, a.symbolCount = pickChangeSymbol(a)
	case a.locDeleted >= 15 && a.locAdded*3 < a.locDeleted*2:
		a.verb = "simplify"
		a.symbolHead, a.symbolCount = pickChangeSymbol(a)
	default:
		switch a.prefix {
		case "docs", "style", "chore", "test":
			a.verb = "update"
		default:
			a.verb = "refactor"
		}
		a.symbolHead, a.symbolCount = pickChangeSymbol(a)
	}
}

func isModifiedStatus(status string) bool {
	return status != "" && status != "A" && status != "??" && status != "D" && status != "R"
}

// pickAddedSymbolForFile returns only symbols introduced in path. The
// dominant-file decision must not be disabled merely because a small,
// unrelated new file exports a type or function.
func (a *commitChangeAnalysis) pickAddedSymbolForFile(path string) (string, int) {
	if path == "" {
		return "", 0
	}
	for _, group := range []struct {
		items []string
		sites map[string]string
	}{
		{a.routes, a.addedRouteSites},
		{a.types, a.addedTypeSites},
		{a.funcs, a.addedFuncSites},
	} {
		var head string
		count := 0
		for _, item := range group.items {
			if group.sites[item] != path {
				continue
			}
			if head == "" {
				head = item
			}
			count++
		}
		if head != "" {
			return head, count
		}
	}
	return "", 0
}

// pickAddSymbol は「何が新規追加されたか」の主辞を選ぶ。API ルートはユーザー可視で
// 最も情報量が高いので最優先、次に型 → 関数 → 追加されたファイルの順で拾う。
func pickAddSymbol(a *commitChangeAnalysis) (string, int) {
	if len(a.routes) > 0 {
		return a.routes[0], len(a.routes)
	}
	if len(a.types) > 0 {
		return a.types[0], len(a.types)
	}
	if len(a.funcs) > 0 {
		return a.funcs[0], len(a.funcs)
	}
	if len(a.added) > 0 {
		return baseName(a.added[0]), len(a.added)
	}
	return "", 0
}

func pickRemoveSymbol(a *commitChangeAnalysis) (string, int) {
	if len(a.removedRts) > 0 {
		return a.removedRts[0], len(a.removedRts)
	}
	if len(a.deleted) > 0 {
		return baseName(a.deleted[0]), len(a.deleted)
	}
	if len(a.deletedFuncs) > 0 {
		return a.deletedFuncs[0], len(a.deletedFuncs)
	}
	return "", 0
}

// pickChangeSymbol は「特定のシンボル追加/削除ではないが何かは変わった」ケースで
// subject に載せる主辞を選ぶ。関数名 / 型名 > 変更ファイル basename > 追加/削除
// ファイル basename の順で拾う。
func pickChangeSymbol(a *commitChangeAnalysis) (string, int) {
	if len(a.funcs) > 0 {
		return a.funcs[0], len(a.funcs)
	}
	if len(a.types) > 0 {
		return a.types[0], len(a.types)
	}
	if len(a.modified) > 0 {
		if a.dominant != "" && isModifiedStatus(a.dominantStatus) {
			return baseName(a.dominant), len(a.modified)
		}
		return baseName(a.modified[0]), len(a.modified)
	}
	if len(a.added) > 0 {
		return baseName(a.added[0]), len(a.added)
	}
	if len(a.deleted) > 0 {
		return baseName(a.deleted[0]), len(a.deleted)
	}
	if len(a.renamePairs) > 0 {
		return a.renamePairs[0], len(a.renamePairs)
	}
	return "", 0
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
	seenDelFunc := map[string]struct{}{}
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
			a.locAdded++
			if reHandleErrGo.MatchString(line) || reHandleThrow.MatchString(line) || reHandleCatch.MatchString(line) {
				a.handleAdds++
			}
			if route := extractRoute(line); route != "" {
				addUnique(&a.routes, seenRoute, route)
				recordAddedSite(&a.addedRouteSites, route, cur)
			}
			if strings.HasSuffix(cur, ".go") {
				if m := reAddedGoFunc.FindStringSubmatch(line); m != nil {
					addUnique(&a.funcs, seenFunc, m[1])
					recordAddedSite(&a.addedFuncSites, m[1], cur)
				}
				if m := reAddedGoType.FindStringSubmatch(line); m != nil {
					addUnique(&a.types, seenType, m[1])
					recordAddedSite(&a.addedTypeSites, m[1], cur)
				}
			}
			if isJSSourcePath(cur) {
				for _, re := range []*regexp.Regexp{reAddedJSFunc, reAddedJSConst, reAddedJSClass} {
					if m := re.FindStringSubmatch(line); m != nil {
						addUnique(&a.funcs, seenFunc, m[1])
						recordAddedSite(&a.addedFuncSites, m[1], cur)
						break
					}
				}
				if m := reAddedJSType.FindStringSubmatch(line); m != nil {
					addUnique(&a.types, seenType, m[1])
					recordAddedSite(&a.addedTypeSites, m[1], cur)
				}
			}
			if strings.HasSuffix(cur, ".py") {
				if m := reAddedPyFunc.FindStringSubmatch(line); m != nil {
					addUnique(&a.funcs, seenFunc, m[1])
					recordAddedSite(&a.addedFuncSites, m[1], cur)
				}
				if m := reAddedPyClass.FindStringSubmatch(line); m != nil {
					addUnique(&a.types, seenType, m[1])
					recordAddedSite(&a.addedTypeSites, m[1], cur)
				}
			}
			if isI18nSourcePath(cur) && reAddedI18n.MatchString(line) {
				a.i18nKeys++
			}
		case isDel:
			a.locDeleted++
			if route := extractRoute(line); route != "" {
				addUnique(&a.removedRts, seenRmRoute, route)
			}
			if strings.HasSuffix(cur, ".go") {
				if m := reDeletedGoFunc.FindStringSubmatch(line); m != nil {
					addUnique(&a.deletedFuncs, seenDelFunc, m[1])
					if a.deletedFuncSites == nil {
						a.deletedFuncSites = map[string]string{}
					}
					if _, ok := a.deletedFuncSites[m[1]]; !ok {
						a.deletedFuncSites[m[1]] = cur
					}
				}
			}
		}
	}
	// 追加・削除の両方に現れたルートは「変更（行移動）」であって新規/削除ではないため、
	// 両側から取り除く。
	a.routes = filterOutSet(a.routes, seenRmRoute)
	a.removedRts = filterOutSet(a.removedRts, seenRoute)
}

func extractRoute(line string) string {
	for _, re := range []*regexp.Regexp{reGoRoute, reJSRoute, rePythonRoute} {
		if m := re.FindStringSubmatch(line); m != nil {
			return m[1]
		}
	}
	return ""
}

func isJSSourcePath(path string) bool {
	lower := strings.ToLower(path)
	for _, ext := range []string{".ts", ".tsx", ".js", ".jsx", ".mts", ".mjs", ".cjs"} {
		if strings.HasSuffix(lower, ext) {
			return true
		}
	}
	return false
}

func isI18nSourcePath(path string) bool {
	lower := strings.ToLower(strings.ReplaceAll(path, "\\", "/"))
	if !strings.Contains(lower, "i18n") && !strings.Contains(lower, "locales") {
		return false
	}
	return strings.HasSuffix(lower, ".json") || strings.HasSuffix(lower, ".ts") || strings.HasSuffix(lower, ".js")
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

// subjectLine は Conventional Commits ハイブリッド型で subject を組む：
//   JA: "<type>(<scope>): <symbol> を<動詞>"（handle/rename は助詞と語尾を個別処理）
//   EN: "<type>(<scope>): <verb> <symbol>"
// scope が prefix と一致・空・prefix が既に括弧付きの場合は (scope) を付けない。
// 変更が全く検出できないレアケースは無情報の "変更なし / no changes" にフォールバック。
func (a commitChangeAnalysis) subjectLine(ja bool) string {
	head := prefixWithScope(a.prefix, a.scope)
	if a.symbolHead == "" {
		if ja {
			return head + ": 変更なし"
		}
		return head + ": no changes"
	}
	if ja {
		subject := head + ": " + jaVerbPhrase(a.verb, withMoreJa(a.symbolHead, a.symbolCount))
		if a.verb == "change" && len(a.added) > 0 {
			subject += "（" + addedFileSummaryJa(a.added) + " 新規）"
		}
		return subject
	}
	subject := head + ": " + enVerbPhrase(a.verb, withMoreEn(a.symbolHead, a.symbolCount))
	if a.verb == "change" && len(a.added) > 0 {
		subject += " (new: " + addedFileSummaryEn(a.added) + ")"
	}
	return subject
}

func recordAddedSite(sites *map[string]string, symbol, path string) {
	if symbol == "" || path == "" {
		return
	}
	if *sites == nil {
		*sites = map[string]string{}
	}
	if _, exists := (*sites)[symbol]; !exists {
		(*sites)[symbol] = path
	}
}

func addedFileSummaryJa(paths []string) string {
	head := baseName(paths[0])
	if len(paths) == 1 {
		return head
	}
	return fmt.Sprintf("%s ほか %d 件", head, len(paths)-1)
}

func addedFileSummaryEn(paths []string) string {
	head := baseName(paths[0])
	if len(paths) == 1 {
		return head
	}
	return fmt.Sprintf("%s (+%d more)", head, len(paths)-1)
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

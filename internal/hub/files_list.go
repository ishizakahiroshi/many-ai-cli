package hub

import (
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
)

// filesListItem は /api/files-list の各ファイルエントリ。
type filesListItem struct {
	Path    string    `json:"path"`
	Rel     string    `json:"rel"`
	Name    string    `json:"name"`
	Type    string    `json:"type,omitempty"`
	Size    int64     `json:"size"`
	Mtime   time.Time `json:"mtime"`
	Summary string    `json:"summary"`
}

// filesListResp は /api/files-list のレスポンス。
type filesListResp struct {
	Root      string          `json:"root"`
	Exists    bool            `json:"exists"`
	Truncated bool            `json:"truncated"`
	Items     []filesListItem `json:"items"`
}

const (
	filesMaxDepth   = 8
	filesMaxItems   = 2000
	filesMaxReadLen = 32 * 1024 // 32 KiB
	filesSummaryLen = 200
)

// 走査時にスキップする重量級ディレクトリ（隠しディレクトリは別途 "." 接頭辞で除外）。
var filesSkipDirs = map[string]bool{
	"node_modules": true,
	"vendor":       true,
	"target":       true,
	"dist":         true,
	"build":        true,
	"out":          true,
	"__pycache__":  true,
}

// handleFilesList は GET /api/files-list を処理する。
// ?session=<id> でセッション cwd を使用、省略時は Hub 起動時 cwd。
// ?root=<absPath> で列挙起点を指定（省略時: <cwd>/docs/local）。
// ?token=<token> 必須。
func (s *Server) handleFilesList(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	// cwd の決定: ?session=<id> があればそのセッションの CWD を使用
	cwd := s.cwdForRequest(r)

	// ?root=<absPath> の処理
	var filesRoot string
	if rootParam := r.URL.Query().Get("root"); rootParam != "" {
		// 絶対パスのみ受け付ける
		if !filepath.IsAbs(rootParam) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "root must be an absolute path")
			return
		}
		// git ルートを検出して許可範囲を確定
		gitRoot := findGitRoot(cwd)
		// path traversal 防止: 許可ルートは cwd または gitRoot
		allowed, err := isPathUnderAllowedRoots(rootParam, cwd, gitRoot)
		if err != nil || !allowed {
			writeJSONError(w, http.StatusForbidden, "forbidden", "path is outside allowed roots")
			return
		}
		filesRoot = rootParam
	} else {
		filesRoot = filepath.Join(cwd, "docs", "local")
	}

	// ルートが存在しない場合
	info, err := os.Stat(filesRoot) // #nosec G703 -- isPathUnderAllowedRoots 検証済み、または cwd 配下の既定パス
	if err != nil || !info.IsDir() {
		writeJSON(w, filesListResp{
			Root:      filesRoot,
			Exists:    false,
			Truncated: false,
			Items:     []filesListItem{},
		})
		return
	}

	items, truncated := walkFilesLocal(filesRoot, cwd)

	resp := filesListResp{
		Root:      filesRoot,
		Exists:    true,
		Truncated: truncated,
		Items:     items,
	}
	writeJSON(w, resp)
}

// walkFilesLocal は filesRoot 以下を再帰走査し filesListItem スライスを返す。
// 件数上限 2000・深さ上限 8・隠しディレクトリ + filesSkipDirs スキップ・シンボリックリンク非追跡。
// 全ファイル・ディレクトリ対象（拡張子フィルタなし）。summary は text 系ファイルのみ抽出。
// mtime 降順ソート済み。
func walkFilesLocal(filesRoot, cwd string) ([]filesListItem, bool) {
	var items []filesListItem
	truncated := false

	var walk func(dir string, depth int)
	walk = func(dir string, depth int) {
		if depth > filesMaxDepth {
			return
		}
		entries, err := os.ReadDir(dir)
		if err != nil {
			return
		}
		for _, e := range entries {
			name := e.Name()
			// 隠しエントリ（. 始まり）と重量級ディレクトリをスキップ
			if strings.HasPrefix(name, ".") {
				continue
			}
			if e.IsDir() && filesSkipDirs[name] {
				continue
			}
			fullPath := filepath.Join(dir, name)

			info, err := e.Info()
			if err != nil {
				continue
			}

			relPath, err := filepath.Rel(cwd, fullPath)
			if err != nil {
				relPath = fullPath
			}

			if e.IsDir() {
				if len(items) >= filesMaxItems {
					truncated = true
					return
				}
				items = append(items, filesListItem{
					Path:  fullPath,
					Rel:   relPath,
					Name:  name,
					Type:  "dir",
					Size:  info.Size(),
					Mtime: info.ModTime(),
				})
				walk(fullPath, depth+1)
				continue
			}

			// シンボリックリンクは追跡しない
			if e.Type()&os.ModeSymlink != 0 {
				continue
			}

			// 件数上限チェック
			if len(items) >= filesMaxItems {
				truncated = true
				return
			}

			// summary は text 系ファイルだけ抽出（バイナリは無意味なのでスキップ）
			var summary string
			if isTextFile(fullPath) {
				summary = extractFileSummary(fullPath)
			}

			items = append(items, filesListItem{
				Path:    fullPath,
				Rel:     relPath,
				Name:    name,
				Type:    "file",
				Size:    info.Size(),
				Mtime:   info.ModTime(),
				Summary: summary,
			})
		}
	}

	walk(filesRoot, 1)

	// mtime 降順ソート
	sort.Slice(items, func(i, j int) bool {
		return items[i].Mtime.After(items[j].Mtime)
	})

	return items, truncated
}

// extractFileSummary はファイルから概要を抽出する（plan §3 の fallback チェーン）。
// 先頭 32 KiB のみ読み込む。
func extractFileSummary(path string) string {
	f, err := os.Open(path) // #nosec G703 -- 呼び出し元が許可ルート配下の walk 結果のみを渡す
	if err != nil {
		return ""
	}
	defer f.Close()

	buf := make([]byte, filesMaxReadLen)
	n, err := io.ReadFull(f, buf)
	if err != nil && n == 0 {
		return ""
	}
	content := string(buf[:n])

	// 1. HTML コメント <!-- summary: ... -->
	if s := extractHtmlCommentSummary(content); s != "" {
		return normalizeSummary(s)
	}

	// 2. YAML frontmatter の description / summary / subtitle
	if s := extractFrontmatterSummary(content); s != "" {
		return normalizeSummary(s)
	}

	// 3 & 4. H1 直下の段落 or 本文先頭段落（引用メタ行スキップ）
	if s := extractParagraphSummary(content); s != "" {
		return normalizeSummary(s)
	}

	return ""
}

var reHtmlCommentSummary = regexp.MustCompile(`(?i)<!--\s*summary:\s*(.*?)\s*-->`)

// extractHtmlCommentSummary は <!-- summary: ... --> を抽出する。
func extractHtmlCommentSummary(content string) string {
	m := reHtmlCommentSummary.FindStringSubmatch(content)
	if len(m) < 2 {
		return ""
	}
	return strings.TrimSpace(m[1])
}

// extractFrontmatterSummary は YAML frontmatter から description / summary / subtitle を抽出する。
func extractFrontmatterSummary(content string) string {
	if !strings.HasPrefix(content, "---") {
		return ""
	}
	// frontmatter の終端を探す
	rest := content[3:]
	if len(rest) > 0 && rest[0] == '\r' {
		rest = rest[1:]
	}
	if len(rest) > 0 && rest[0] == '\n' {
		rest = rest[1:]
	}
	end := strings.Index(rest, "\n---")
	if end < 0 {
		return ""
	}
	fm := rest[:end]

	for _, line := range strings.Split(fm, "\n") {
		line = strings.TrimRight(line, "\r")
		for _, key := range []string{"description", "summary", "subtitle"} {
			prefix := key + ":"
			if strings.HasPrefix(strings.ToLower(line), prefix) {
				val := strings.TrimSpace(line[len(prefix):])
				// YAML クオート除去
				if len(val) >= 2 && ((val[0] == '"' && val[len(val)-1] == '"') ||
					(val[0] == '\'' && val[len(val)-1] == '\'')) {
					val = val[1 : len(val)-1]
				}
				if val != "" {
					return val
				}
			}
		}
	}
	return ""
}

// extractParagraphSummary は H1 直下（または本文先頭）の段落を抽出する。
// 引用 > で始まるメタ行はスキップ。
func extractParagraphSummary(content string) string {
	lines := strings.Split(content, "\n")

	// frontmatter をスキップ
	start := 0
	if len(lines) > 0 && strings.HasPrefix(lines[0], "---") {
		for i := 1; i < len(lines); i++ {
			if strings.TrimRight(lines[i], "\r") == "---" {
				start = i + 1
				break
			}
		}
	}

	// H1 を探す
	h1Found := false
	for i := start; i < len(lines); i++ {
		trimmed := strings.TrimRight(lines[i], "\r")
		if strings.HasPrefix(trimmed, "# ") {
			h1Found = true
			start = i + 1
			break
		}
	}

	// H1 直後（または先頭）から最初の有効段落を探す
	// 引用（>）で始まる行、空行、見出し行を読み飛ばして段落を収集
	var paraLines []string
	inPara := false

	_ = h1Found // 分岐は start で制御済み

	for i := start; i < len(lines); i++ {
		trimmed := strings.TrimRight(lines[i], "\r")

		// 空行
		if strings.TrimSpace(trimmed) == "" {
			if inPara {
				break // 段落終了
			}
			continue
		}

		// 見出し行
		if strings.HasPrefix(trimmed, "#") {
			if inPara {
				break
			}
			continue
		}

		// 引用メタ行（> で始まる行）はスキップ
		if strings.HasPrefix(trimmed, ">") {
			if inPara {
				break
			}
			continue
		}

		// コードフェンス
		if strings.HasPrefix(trimmed, "```") || strings.HasPrefix(trimmed, "~~~") {
			if inPara {
				break
			}
			// フェンス内をスキップ
			fence := trimmed[:3]
			i++
			for i < len(lines) {
				l2 := strings.TrimRight(lines[i], "\r")
				if strings.HasPrefix(l2, fence) {
					break
				}
				i++
			}
			continue
		}

		inPara = true
		paraLines = append(paraLines, trimmed)
	}

	return strings.Join(paraLines, " ")
}

var (
	reMdLink     = regexp.MustCompile(`\[([^\]]*)\]\([^)]*\)`)
	reCodeFence  = regexp.MustCompile("(?s)```.*?```")
	reInlineCode = regexp.MustCompile("`[^`]*`")
	reMultiSpace = regexp.MustCompile(`\s+`)
)

// normalizeSummary は概要テキストを正規化する。
// - Markdown リンク [text](url) を text のみに
// - コードフェンス除去（インラインコードも）
// - 改行→空白
// - 連続空白圧縮
// - 200文字トリム（超過時 … 付与）
func normalizeSummary(s string) string {
	// コードフェンス除去
	s = reCodeFence.ReplaceAllString(s, "")
	// インラインコード除去
	s = reInlineCode.ReplaceAllString(s, "")
	// Markdown リンクを text のみに
	s = reMdLink.ReplaceAllString(s, "$1")
	// 改行→空白
	s = strings.NewReplacer("\r\n", " ", "\r", " ", "\n", " ").Replace(s)
	// 連続空白圧縮
	s = reMultiSpace.ReplaceAllString(s, " ")
	s = strings.TrimSpace(s)
	// 200文字トリム
	runes := []rune(s)
	if len(runes) > filesSummaryLen {
		s = string(runes[:filesSummaryLen]) + "…"
	}
	return s
}

// findGitRoot は dir から上方向に .git ディレクトリを探してそのパスを返す。
// 見つからなければ dir 自身を返す。
func findGitRoot(dir string) string {
	current := filepath.Clean(dir)
	for {
		gitDir := filepath.Join(current, ".git")
		if info, err := os.Stat(gitDir); err == nil && info.IsDir() {
			return current
		}
		parent := filepath.Dir(current)
		if parent == current {
			// ファイルシステムルートに達した
			break
		}
		current = parent
	}
	return dir
}

// evalSymlinksViaSelf は path を EvalSymlinks で解決する。
// path 自体が存在しない場合は親ディレクトリを解決して basename を付け直す
// （move/rename/delete のように「これから作るパス」の symlink 脱出を防ぐため）。
// 解決できない場合は cleaned のまま返し、ok=false を返す。
func evalSymlinksViaSelf(cleaned string) (resolved string, ok bool) {
	if r, err := filepath.EvalSymlinks(cleaned); err == nil {
		return r, true
	}
	// 自身が未存在の場合: 親を解決して basename を付け直す
	parent := filepath.Dir(cleaned)
	base := filepath.Base(cleaned)
	if rp, err := filepath.EvalSymlinks(parent); err == nil {
		return filepath.Join(rp, base), true
	}
	return cleaned, false
}

// isPathUnderAllowedRoots は target パスが allowedRoots のいずれかの配下にあるか検証する。
// path traversal 防止のため:
//  1. filepath.Clean で正規化
//  2. filepath.EvalSymlinks で自身または親を解決（symlink 経由の脱出を塞ぐ）
//  3. 判定は resolved ベースに統一（cleaned フォールバックは使わない）
//  4. 解決できない場合は拒否寄り（ok=false）
func isPathUnderAllowedRoots(target string, allowedRoots ...string) (bool, error) {
	cleaned := filepath.Clean(target)

	// symlink を解決（対象が未存在でも親経由で解決する）
	resolved, ok := evalSymlinksViaSelf(cleaned)
	if !ok {
		// 完全に解決できない場合は cleaned で判定（移動先の新規ファイル等）
		resolved = cleaned
	}

	for _, root := range allowedRoots {
		if root == "" {
			continue
		}
		cleanedRoot := filepath.Clean(root)
		resolvedRoot, _ := evalSymlinksViaSelf(cleanedRoot)

		// resolved ベースのみで判定（cleaned フォールバックは使わない）
		if isUnder(resolved, resolvedRoot) {
			return true, nil
		}
	}
	return false, nil
}

// isUnder は target が base の配下（または base 自身）かを返す。
// Windows のドライブレターはケースインセンシティブで比較する。
func isUnder(target, base string) bool {
	if target == "" || base == "" {
		return false
	}
	// 末尾セパレータを統一して比較
	base = filepath.Clean(base)
	target = filepath.Clean(target)

	// Windows: ケースインセンシティブ比較のため両辺を小文字化してから Rel
	cmpBase := base
	cmpTarget := target
	if isWindowsPath(base) || isWindowsPath(target) {
		cmpBase = strings.ToLower(base)
		cmpTarget = strings.ToLower(target)
	}

	rel, err := filepath.Rel(cmpBase, cmpTarget)
	if err != nil {
		return false
	}
	// ".." または "../..." の相対パスだけ base の外側として扱う。
	if rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return false
	}

	// Windows: ドライブレター違いの場合 Rel が絶対パスを返すことがある
	if filepath.IsAbs(rel) {
		return false
	}

	return true
}

// isWindowsPath は path が Windows ドライブレター形式（例: C:\...）かを判定する。
func isWindowsPath(path string) bool {
	return len(path) >= 2 && path[1] == ':'
}

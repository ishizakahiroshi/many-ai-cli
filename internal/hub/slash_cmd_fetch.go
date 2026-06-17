package hub

import (
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
	"many-ai-cli/internal/config"
)

// SlashCmd はスラッシュコマンドの名前と説明。
type SlashCmd struct {
	Cmd  string `json:"cmd"`
	Desc string `json:"desc"`
	Kind string `json:"kind,omitempty"`
	Name string `json:"name,omitempty"`
	Path string `json:"-"`
}

type slashCmdCacheEntry struct {
	cmds      []SlashCmd
	fetchedAt time.Time
	sourceURL string
}

type skillSearchContext struct {
	HomeDir   string
	CodexHome string
	ClaudeDir string
}

const slashCmdCacheTTL = 24 * time.Hour

var (
	// markdown テーブル行: | `/cmd` | description |
	tableRowRe = regexp.MustCompile("\\|\\s*`?(/[a-z][a-z0-9_-]*)(?:[^`|\\n]*)`?\\s*\\|([^|\\n]+)")
	// バッククオートリスト: - `/cmd` - description
	listItemRe = regexp.MustCompile("(?m)^[ \\t]*[-*][ \\t]+`(/[a-z][a-z0-9_-]*)(?:[^`]*)`[ \\t]*[-–—:]+[ \\t]*(.+)")
	// 裸のリスト: - /cmd - description
	bareListRe = regexp.MustCompile(`(?m)^[ \t]*[-*][ \t]+(/[a-z][a-z0-9_-]+)[ \t]+[-–—:]+[ \t]*(.+)`)
	// ドキュメントのプレーンテキスト化行: /cmd Description...
	plainLineRe = regexp.MustCompile("(?m)^[ \\t]*`?(/[a-z][a-z0-9_-]*)(?:\\s+[^`\\n]*)?`?[ \\t]+([^\\n]+)")

	// description サニタイズ用
	mdLinkRe   = regexp.MustCompile(`\[([^\]]*)\]\([^)]*\)`)         // [text](url) → text
	mdImgRe    = regexp.MustCompile(`!\[([^\]]*)\]\([^)]*\)`)        // ![alt](url) → alt
	mdBoldRe   = regexp.MustCompile(`\*\*([^*]+)\*\*|__([^_]+)__`)   // **text** / __text__ → text
	mdItalicRe = regexp.MustCompile(`(?:\*([^*\n]+)\*|_([^_\n]+)_)`) // *text* / _text_ → text
	mdCodeRe   = regexp.MustCompile("`([^`]+)`")                     // `text` → text
	// 残った半端な括弧／角括弧の組
	mdStrayBracketRe = regexp.MustCompile(`\[[^\]]*\]|\([^)]*\)`)
	wsRe             = regexp.MustCompile(`\s+`)

	slashCmdRemoteHostAllowlist = map[string]bool{
		"raw.githubusercontent.com": true,
	}
)

// cleanDescMarkdown は description から markdown 装飾を除去してプレーン化する。
func cleanDescMarkdown(s string) string {
	s = mdImgRe.ReplaceAllString(s, "$1")
	s = mdLinkRe.ReplaceAllString(s, "$1")
	for i := 0; i < 3; i++ {
		s = mdBoldRe.ReplaceAllStringFunc(s, func(m string) string {
			sub := mdBoldRe.FindStringSubmatch(m)
			if sub[1] != "" {
				return sub[1]
			}
			return sub[2]
		})
	}
	s = mdItalicRe.ReplaceAllStringFunc(s, func(m string) string {
		sub := mdItalicRe.FindStringSubmatch(m)
		if sub[1] != "" {
			return sub[1]
		}
		return sub[2]
	})
	s = mdCodeRe.ReplaceAllString(s, "$1")
	// `[Skill](/...)` のように角括弧／丸括弧の対応が崩れて残った断片を除去
	s = mdStrayBracketRe.ReplaceAllString(s, "")
	// HTML タグが混入していた場合に備えてゆるく剥がす
	s = strings.ReplaceAll(s, "<br>", " ")
	s = strings.ReplaceAll(s, "<br/>", " ")
	s = strings.ReplaceAll(s, "<br />", " ")
	s = wsRe.ReplaceAllString(s, " ")
	return strings.TrimSpace(s)
}

func fetchAndParseSlashCmds(source string) ([]SlashCmd, error) {
	body, err := readSlashCmdSource(source)
	if err != nil {
		return nil, err
	}
	return parseSlashCmdsFromMarkdown(string(body)), nil
}

const slashCmdLocalMaxBytes = 2 << 20 // 2MB

func validateSlashCmdSource(source string) error {
	source = strings.TrimSpace(source)
	if source == "" {
		return nil
	}
	if strings.Contains(source, "://") {
		u, err := url.Parse(source)
		if err != nil || u.Hostname() == "" {
			return fmt.Errorf("invalid URL")
		}
		if strings.ToLower(u.Scheme) != "https" {
			return fmt.Errorf("URL scheme must be https")
		}
		host := strings.TrimSuffix(strings.ToLower(u.Hostname()), ".")
		if isBlockedNetworkHost(host) {
			return fmt.Errorf("URL host is not allowed")
		}
		if !slashCmdRemoteHostAllowlist[host] {
			return fmt.Errorf("URL host %q is not allowed", host)
		}
		if u.User != nil {
			return fmt.Errorf("URL credentials are not allowed")
		}
		return nil
	}

	if !filepath.IsAbs(source) {
		return fmt.Errorf("local source path must be absolute")
	}
	cfgDir, err := config.Dir()
	if err != nil {
		return err
	}
	allowed, err := isPathUnderAllowedRoots(source, cfgDir)
	if err != nil {
		return err
	}
	if !allowed {
		return fmt.Errorf("local source path must be under %s", cfgDir)
	}
	return nil
}

func readSlashCmdSource(source string) ([]byte, error) {
	source = strings.TrimSpace(source)
	if source == "" {
		return nil, fmt.Errorf("source is empty")
	}
	if err := validateSlashCmdSource(source); err != nil {
		return nil, err
	}
	if !strings.Contains(source, "://") {
		// ローカルファイル: サイズ上限 + ディレクトリ/デバイス拒否
		info, err := os.Stat(source)
		if err != nil {
			return nil, fmt.Errorf("stat %s: %w", source, err)
		}
		if !info.Mode().IsRegular() {
			return nil, fmt.Errorf("source %s is not a regular file", source)
		}
		f, err := os.Open(source)
		if err != nil {
			return nil, fmt.Errorf("open %s: %w", source, err)
		}
		defer f.Close()
		body, err := io.ReadAll(io.LimitReader(f, slashCmdLocalMaxBytes+1))
		if err != nil {
			return nil, err
		}
		if int64(len(body)) > slashCmdLocalMaxBytes {
			return nil, fmt.Errorf("source %s exceeds %d bytes", source, slashCmdLocalMaxBytes)
		}
		return body, nil
	}
	client := makeExternalHTTPClient(15 * time.Second)
	resp, err := client.Get(source)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		// エラーページを読み捨てて早期リターン（ボディは読まない）
		return nil, fmt.Errorf("fetch %s: %s", source, resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 2<<20)) // 2MB
	if err != nil {
		return nil, err
	}
	return body, nil
}

func parseSlashCmdsFromMarkdown(text string) []SlashCmd {
	seen := map[string]bool{}
	var result []SlashCmd

	add := func(cmd, desc string) {
		cmd = strings.TrimSpace(cmd)
		desc = strings.TrimSpace(desc)
		desc = strings.TrimRight(desc, " |")
		desc = cleanDescMarkdown(desc)
		desc = strings.TrimRight(desc, " .")
		if cmd == "" || seen[cmd] {
			return
		}
		seen[cmd] = true
		result = append(result, SlashCmd{Cmd: cmd, Desc: desc})
	}

	for _, m := range tableRowRe.FindAllStringSubmatch(text, -1) {
		add(m[1], m[2])
	}
	for _, m := range listItemRe.FindAllStringSubmatch(text, -1) {
		add(m[1], m[2])
	}
	for _, m := range bareListRe.FindAllStringSubmatch(text, -1) {
		add(m[1], m[2])
	}
	for _, m := range plainLineRe.FindAllStringSubmatch(text, -1) {
		add(m[1], m[2])
	}

	return result
}

type skillFrontmatter struct {
	Name          string `yaml:"name"`
	Description   string `yaml:"description"`
	UserInvokable *bool  `yaml:"user-invokable"`
}

const skillFrontmatterMaxBytes = 64 << 10
const skillDiscoveryMaxItems = 500

func discoverSkillSlashCmds(provider string, ctx skillSearchContext) []SlashCmd {
	if provider != "claude" && provider != "codex" {
		return nil
	}
	roots := skillSearchRoots(provider, ctx)
	seenPath := map[string]bool{}
	var out []SlashCmd
	for _, root := range roots {
		if root == "" || seenPath[filepath.Clean(root)] {
			continue
		}
		seenPath[filepath.Clean(root)] = true
		info, err := os.Stat(root)
		if err != nil || !info.IsDir() {
			continue
		}
		_ = filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
			if err != nil {
				return nil
			}
			if len(out) >= skillDiscoveryMaxItems {
				return filepath.SkipAll
			}
			if d.IsDir() {
				switch d.Name() {
				case ".git", "node_modules", "__pycache__":
					return filepath.SkipDir
				}
				return nil
			}
			if !strings.EqualFold(d.Name(), "SKILL.md") {
				return nil
			}
			cmd, ok := readSkillSlashCmd(provider, path)
			if ok {
				out = append(out, cmd)
			}
			return nil
		})
	}
	return out
}

func skillSearchRoots(provider string, ctx skillSearchContext) []string {
	home := strings.TrimSpace(ctx.HomeDir)
	if home == "" {
		var err error
		home, err = os.UserHomeDir()
		if err != nil {
			return nil
		}
	}
	switch provider {
	case "codex":
		codexHome := strings.TrimSpace(ctx.CodexHome)
		if codexHome == "" {
			codexHome = strings.TrimSpace(os.Getenv("CODEX_HOME"))
		}
		if codexHome == "" {
			codexHome = filepath.Join(home, ".codex")
		}
		return []string{
			filepath.Join(codexHome, "skills"),
			filepath.Join(codexHome, "plugins", "cache"),
		}
	case "claude":
		claudeHome := strings.TrimSpace(ctx.ClaudeDir)
		if claudeHome == "" {
			claudeHome = strings.TrimSpace(os.Getenv("CLAUDE_CONFIG_DIR"))
		}
		if claudeHome == "" {
			claudeHome = filepath.Join(home, ".claude")
		}
		return []string{
			filepath.Join(claudeHome, "skills"),
			filepath.Join(claudeHome, "plugins"),
		}
	default:
		return nil
	}
}

func readSkillSlashCmd(provider, path string) (SlashCmd, bool) {
	meta, ok := readSkillFrontmatter(path)
	if !ok {
		return SlashCmd{}, false
	}
	if meta.UserInvokable != nil && !*meta.UserInvokable {
		return SlashCmd{}, false
	}
	name := strings.TrimSpace(meta.Name)
	if name == "" {
		name = filepath.Base(filepath.Dir(path))
	}
	if !isSafeSkillName(name) {
		return SlashCmd{}, false
	}
	desc := cleanDescMarkdown(meta.Description)
	if desc == "" {
		desc = "Skill"
	} else {
		desc = "Skill. " + desc
	}
	return SlashCmd{
		Cmd:  skillCommandForProvider(provider, name),
		Desc: desc,
		Kind: "skill",
		Name: name,
		Path: path,
	}, true
}

func readSkillFrontmatter(path string) (skillFrontmatter, bool) {
	f, err := os.Open(path)
	if err != nil {
		return skillFrontmatter{}, false
	}
	defer f.Close()
	body, err := io.ReadAll(io.LimitReader(f, skillFrontmatterMaxBytes))
	if err != nil {
		return skillFrontmatter{}, false
	}
	text := strings.ReplaceAll(string(body), "\r\n", "\n")
	text = strings.ReplaceAll(text, "\r", "\n")
	if !strings.HasPrefix(text, "---\n") {
		return skillFrontmatter{}, false
	}
	end := strings.Index(text[4:], "\n---")
	if end < 0 {
		return skillFrontmatter{}, false
	}
	raw := text[4 : 4+end]
	var meta skillFrontmatter
	if err := yaml.Unmarshal([]byte(raw), &meta); err != nil {
		return skillFrontmatter{}, false
	}
	return meta, true
}

func isSafeSkillName(name string) bool {
	if name == "" || len(name) > 128 {
		return false
	}
	for i, r := range name {
		ok := (r >= 'a' && r <= 'z') ||
			(r >= 'A' && r <= 'Z') ||
			(r >= '0' && r <= '9') ||
			r == '-' || r == '_' || r == '.' || r == ':'
		if !ok {
			return false
		}
		if i == 0 && !(r >= 'a' && r <= 'z') && !(r >= 'A' && r <= 'Z') && !(r >= '0' && r <= '9') {
			return false
		}
	}
	return true
}

func skillCommandForProvider(provider, name string) string {
	if provider == "codex" {
		return "$" + name
	}
	return "/" + name
}

func mergeSkillSlashCmds(provider string, cmds, skills []SlashCmd) []SlashCmd {
	if len(skills) == 0 {
		return cmds
	}
	if provider == "claude" {
		return dedupeSlashCmds(append(skills, cmds...))
	}
	return dedupeSlashCmds(append(cmds, skills...))
}

func dedupeSlashCmds(cmds []SlashCmd) []SlashCmd {
	seen := map[string]bool{}
	out := make([]SlashCmd, 0, len(cmds))
	for _, cmd := range cmds {
		key := strings.TrimSpace(cmd.Cmd)
		if key == "" || seen[key] {
			continue
		}
		seen[key] = true
		out = append(out, cmd)
	}
	return out
}

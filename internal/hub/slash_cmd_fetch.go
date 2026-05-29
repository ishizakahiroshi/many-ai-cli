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

	"any-ai-cli/internal/config"
)

// SlashCmd はスラッシュコマンドの名前と説明。
type SlashCmd struct {
	Cmd  string `json:"cmd"`
	Desc string `json:"desc"`
}

type slashCmdCacheEntry struct {
	cmds      []SlashCmd
	fetchedAt time.Time
	sourceURL string
}

const slashCmdCacheTTL = 24 * time.Hour

var (
	// markdown テーブル行: | `/cmd` | description |
	tableRowRe = regexp.MustCompile("\\|\\s*`?(/[a-z][a-z0-9_-]*)(?:[^`|\\n]*)`?\\s*\\|([^|\\n]+)")
	// バッククオートリスト: - `/cmd` - description
	listItemRe = regexp.MustCompile("(?m)^[ \\t]*[-*][ \\t]+`(/[a-z][a-z0-9_-]*)(?:[^`]*)`[ \\t]*[-–—:]+[ \\t]*(.+)")
	// 裸のリスト: - /cmd - description
	bareListRe = regexp.MustCompile("(?m)^[ \\t]*[-*][ \\t]+(/[a-z][a-z0-9_-]+)[ \\t]+[-–—:]+[ \\t]*(.+)")
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
		return io.ReadAll(io.LimitReader(f, slashCmdLocalMaxBytes))
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

package hub

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"regexp"
	"strings"
	"time"
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

func readSlashCmdSource(source string) ([]byte, error) {
	if source == "" {
		return nil, fmt.Errorf("source is empty")
	}
	if !strings.HasPrefix(source, "http://") && !strings.HasPrefix(source, "https://") {
		return os.ReadFile(source)
	}
	client := &http.Client{Timeout: 15 * time.Second}
	resp, err := client.Get(source)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, 2<<20)) // 2MB
	if err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("fetch %s: %s", source, resp.Status)
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

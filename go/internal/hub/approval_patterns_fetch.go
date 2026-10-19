package hub

import (
	"regexp"
	"strings"
)

// バッククオート囲みのリストアイテム: - `pattern text`
var approvalPatternLineRe = regexp.MustCompile("(?m)^[ \\t]*[-*][ \\t]+`([^`]+)`[ \\t]*$")

// fetchAndParseApprovalPatterns は md ファイル（URL or ローカルパス）を取得してパースする。
// 取得規則は readSlashCmdSource を流用（15s timeout / 2MB 上限）。
func fetchAndParseApprovalPatterns(source string) ([]string, error) {
	body, err := readSlashCmdSource(source)
	if err != nil {
		return nil, err
	}
	return parseApprovalPatternsFromMarkdown(string(body)), nil
}

// parseApprovalPatternsFromMarkdown はバッククオート囲みのリストアイテムから
// 文言を抽出する。空文字・重複は除外する。
func parseApprovalPatternsFromMarkdown(text string) []string {
	seen := map[string]bool{}
	var result []string
	for _, m := range approvalPatternLineRe.FindAllStringSubmatch(text, -1) {
		v := strings.TrimSpace(m[1])
		if v == "" || seen[v] {
			continue
		}
		seen[v] = true
		result = append(result, v)
	}
	return result
}

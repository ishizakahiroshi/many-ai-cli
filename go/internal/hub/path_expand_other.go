//go:build !windows

package hub

// expandPathEntries は Windows でのみ意味を持つ `%VAR%` 再展開のスタブ。
// macOS / Linux では PATH の `%VAR%` 形式は存在しないため no-op で返す。
func expandPathEntries(entries []string) []string {
	return entries
}

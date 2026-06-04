package launcher

import "strings"

// ShellQuote wraps s in single quotes for safe POSIX shell expansion via
// `bash -lc '...'`. Embedded single quotes use the POSIX
// close/quote/reopen idiom.
func ShellQuote(s string) string {
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

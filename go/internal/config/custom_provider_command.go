package config

import (
	"fmt"
	"strings"
)

// SplitCommandLine splits a CustomProvider.Command string into argv without
// ever invoking an OS shell. This is the single source of truth for the
// rule — internal/wrapper (actual process launch) and internal/doctor
// (PATH-existence probe) both call it, and README.md's "Custom providers"
// section documents these same rules for users. Do not change the rules
// here without updating both.
//
//  1. ASCII space/tab delimit tokens. Runs of delimiters collapse to one;
//     leading/trailing delimiters are discarded.
//  2. A double-quoted span is one token, or part of one — quoting can start
//     and end mid-token (`--path="C:\a b\c"` -> `--path=C:\a b\c`). The
//     surrounding quotes are not part of the result.
//  3. `""` inside a quoted span is a literal `"` (CSV-style escaping).
//  4. `\` is always a literal character, never an escape. This keeps
//     Windows paths (`C:\a\b.exe`) intact without special-casing them.
//  5. `'` has no special meaning; it is an ordinary character.
//  6. Nothing is expanded or interpreted: environment variables (`$X`,
//     `%X%`), `~`, globs, and shell operators (`|` `&&` `;` `>` `<`) all
//     pass through as literal argv content, because this never reaches a
//     real shell.
//  7. An unterminated quote, a control character (other than the space/tab
//     delimiters) anywhere in s, or zero resulting tokens are errors.
func SplitCommandLine(s string) ([]string, error) {
	var tokens []string
	var cur strings.Builder
	tokenStarted := false
	inQuotes := false

	flush := func() {
		if tokenStarted {
			tokens = append(tokens, cur.String())
			cur.Reset()
			tokenStarted = false
		}
	}

	runes := []rune(s)
	for i := 0; i < len(runes); i++ {
		r := runes[i]
		if inQuotes {
			if r == '"' {
				if i+1 < len(runes) && runes[i+1] == '"' {
					cur.WriteRune('"')
					i++
					continue
				}
				inQuotes = false
				continue
			}
			if isCommandControlRune(r) {
				return nil, fmt.Errorf("custom provider command contains a control character (0x%02x)", r)
			}
			cur.WriteRune(r)
			continue
		}
		switch r {
		case ' ', '\t':
			flush()
		case '"':
			inQuotes = true
			tokenStarted = true
		default:
			if isCommandControlRune(r) {
				return nil, fmt.Errorf("custom provider command contains a control character (0x%02x)", r)
			}
			tokenStarted = true
			cur.WriteRune(r)
		}
	}
	if inQuotes {
		return nil, fmt.Errorf(`custom provider command has an unterminated "`)
	}
	flush()
	if len(tokens) == 0 {
		return nil, fmt.Errorf("custom provider command is empty")
	}
	return tokens, nil
}

// isCommandControlRune reports whether r is a control character SplitCommandLine
// rejects. Space (0x20) and tab (0x09) are handled as delimiters before this
// is ever consulted, so they never reach here.
func isCommandControlRune(r rune) bool {
	return r < 0x20 || r == 0x7f
}

// Argv splits p.Command via SplitCommandLine.
func (p CustomProvider) Argv() ([]string, error) {
	return SplitCommandLine(p.Command)
}

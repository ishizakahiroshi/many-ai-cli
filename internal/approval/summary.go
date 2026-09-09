// Package approval provides the provider-neutral facts used to present and
// decide pending tool approvals.  It deliberately does not make a decision
// about whether an approval may be bypassed; callers can use RiskTier as one
// input to their own policy.
package approval

import (
	"regexp"
	"strings"

	"many-ai-cli/internal/proto"
)

var (
	commandPrefixes = []*regexp.Regexp{
		regexp.MustCompile(`(?i)^\s*(?:run|command|bash command|shell command)\s*:\s*(.+)$`),
		regexp.MustCompile(`(?i)^\s*not in allowlist\s*:\s*(.+)$`),
		regexp.MustCompile(`(?i)^\s*(?:execute|executing)\s+(.+)$`),
	}
	pathPattern  = regexp.MustCompile("(?i)(?:[a-z]:[\\\\/][^\\s'\"`<>|]+|(?:\\.\\.?[\\\\/]|/)[^\\s'\"`<>|]+|\\*\\.[a-z0-9_-]+)")
	spacePattern = regexp.MustCompile(`\s+`)

	// highRiskRe catches destructive commands the literal highSignals list misses:
	// flag-order variants (rm -fr / -Rf / --recursive) and piped installers
	// (curl ... | sh). Matched against the whole command so pipe-based patterns
	// are not split apart by the segment loop below.
	highRiskRe = []*regexp.Regexp{
		regexp.MustCompile(`(?i)\brm\s+(?:-[a-z]*r[a-z]*|--recursive)`),
		regexp.MustCompile(`(?i)\b(?:chmod|chown)\s+(?:-[a-z]*r[a-z]*|--recursive)`),
		regexp.MustCompile(`(?i)\bremove-item\b[^\n]*-recurse`),
		regexp.MustCompile(`(?i)\b(?:curl|wget|iwr|invoke-webrequest)\b[^\n]*\|\s*(?:sudo\s+)?(?:sh|bash|zsh|python[0-9.]*|perl|node|pwsh|powershell|iex)\b`),
		regexp.MustCompile(`(?i)\bgit\s+reset\s+--hard\b`),
		regexp.MustCompile(`(?i)\bgit\s+clean\s+-`),
		regexp.MustCompile(`(?i)\bgit\s+push\b[^\n]*(?:--force|--force-with-lease|\s-f\b)`),
		regexp.MustCompile(`(?i)\b(?:sudo|doas)\s`),
		regexp.MustCompile(`(?i)\b(?:mkfs\S*|shutdown|reboot|halt|poweroff)\b`),
		regexp.MustCompile(`(?i)\bdd\s+[^\n]*\bof=`),
		regexp.MustCompile(`(?i)(?:^|\s)(?:del|rmdir)\s+/s\b`),
		regexp.MustCompile(`(?i)\bformat\s+[a-z]:`),
	}
	// findSideEffectRe catches find actions that can mutate files even when the
	// command begins with the low-risk "find " prefix.
	findSideEffectRe   = regexp.MustCompile(`(?i)\bfind\b[^\n]*\s-(?:exec|execdir|ok|okdir|delete)\b`)
	gitBranchCommandRe = regexp.MustCompile(`(?i)^\s*git\s+branch(?:\s|$)`)
)

// Summarize extracts conservative display facts from an already ANSI-stripped
// approval prompt.  Unknown commands are mid risk: a missing parser match must
// never quietly become low risk for downstream auto-approval consumers.
func Summarize(question, context string) proto.ApprovalSummary {
	command := extractCommand(question, context)
	return proto.ApprovalSummary{
		Command: command,
		Paths:   extractPaths(command + "\n" + context),
		Risk:    ClassifyRisk(command),
		Raw:     strings.TrimSpace(context),
	}
}

// ClassifyRisk is the shared risk-tier contract for approval presentation,
// auto-approval, and outbound notification.  It intentionally keeps only
// three stable values.  Callers must treat unknown/empty commands as mid.
func ClassifyRisk(command string) proto.ApprovalRiskTier {
	value := strings.ToLower(strings.TrimSpace(command))
	if value == "" {
		return proto.ApprovalRiskMid
	}
	highSignals := []string{
		"rm -rf", "rmdir /s", "del /s", "remove-item -recurse", "git reset --hard",
		"git clean -", "push --force", "push -f", "sudo ", "doas ", "mkfs", "dd ",
		"shutdown", "reboot", "format ", "curl |", "wget |", "chmod -r", "chown -r",
	}
	// High-risk patterns are matched against the whole command first (before the
	// segment split) so pipe-based ones like "curl ... | sh" stay intact.
	for _, re := range highRiskRe {
		if re.MatchString(value) {
			return proto.ApprovalRiskHigh
		}
	}
	for _, signal := range highSignals {
		if strings.Contains(value, signal) {
			return proto.ApprovalRiskHigh
		}
	}
	// Read-only prefixes are not enough to classify shell syntax. A redirect,
	// command substitution, find side-effect action, or branch mutation makes
	// the command manual even when it starts with cat, ls, or git branch.
	if HasWriteRedirect(value) || IsGitBranchMutation(value) || findSideEffectRe.MatchString(value) || strings.Contains(value, "$(") || strings.Contains(value, "`") {
		return proto.ApprovalRiskMid
	}
	lowPrefixes := []string{
		"cat ", "head ", "tail ", "ls", "dir", "pwd", "git status", "git diff",
		"git log", "git show", "git branch", "find ", "rg ", "grep ", "type ",
	}
	// Classify each connector-separated segment and take the worst tier. A
	// low-risk leading segment must not make the whole command low.
	sawLow, sawMid := false, false
	for _, seg := range splitRiskSegments(value) {
		seg = strings.TrimSpace(seg)
		if seg == "" {
			continue
		}
		isLow := false
		for _, prefix := range lowPrefixes {
			if seg == strings.TrimSpace(prefix) || strings.HasPrefix(seg, prefix) {
				isLow = true
				break
			}
		}
		if isLow {
			sawLow = true
		} else {
			sawMid = true
		}
	}
	switch {
	case sawMid:
		return proto.ApprovalRiskMid
	case sawLow:
		return proto.ApprovalRiskLow
	default:
		return proto.ApprovalRiskMid
	}
}

// splitRiskSegments breaks a command on shell connectors so each part is
// classified independently. The ampersand in a descriptor duplication such
// as "2>&1" is not a command connector.
func splitRiskSegments(value string) []string {
	segments := make([]string, 0, 2)
	start := 0
	var quote byte
	escaped := false
	for i := 0; i < len(value); i++ {
		ch := value[i]
		if escaped {
			escaped = false
			continue
		}
		if quote == '\'' {
			if ch == '\'' {
				quote = 0
			}
			continue
		}
		if quote == '"' {
			switch ch {
			case '\\':
				escaped = true
			case '"':
				quote = 0
			}
			continue
		}
		switch ch {
		case '\\':
			escaped = true
		case '\'', '"':
			quote = ch
		case ';', '\n', '|':
			segments = append(segments, value[start:i])
			if i+1 < len(value) && value[i+1] == ch {
				i++
			}
			start = i + 1
		case '&':
			if i > 0 && value[i-1] == '>' {
				continue
			}
			segments = append(segments, value[start:i])
			if i+1 < len(value) && value[i+1] == '&' {
				i++
			}
			start = i + 1
		}
	}
	return append(segments, value[start:])
}

// HasWriteRedirect reports an unquoted, unescaped shell output redirect that
// writes to a file. It leaves descriptor duplication and the conventional null
// sinks alone so harmless stderr handling does not make a read-only command
// manual. Commands with other shell syntax remain conservative at mid risk.
func HasWriteRedirect(command string) bool {
	var quote byte
	escaped := false
	for i := 0; i < len(command); i++ {
		ch := command[i]
		if escaped {
			escaped = false
			continue
		}
		if quote == '\'' {
			if ch == '\'' {
				quote = 0
			}
			continue
		}
		if quote == '"' {
			switch ch {
			case '\\':
				escaped = true
			case '"':
				quote = 0
			}
			continue
		}
		switch ch {
		case '\\', '^':
			escaped = true
		case '\'', '"':
			quote = ch
		case '>':
			if !safeRedirectTarget(command, i) {
				return true
			}
		}
	}
	return false
}

func safeRedirectTarget(command string, redirectIndex int) bool {
	start := redirectIndex + 1
	if start < len(command) && command[start] == '>' {
		start++
	}
	for start < len(command) && (command[start] == ' ' || command[start] == '\t') {
		start++
	}
	if start >= len(command) {
		return false
	}
	if command[start] == '&' {
		if start+1 >= len(command) {
			return false
		}
		next := command[start+1]
		return next == '-' || (next >= '0' && next <= '9')
	}
	target := command[start:]
	if target[0] == '\'' || target[0] == '"' {
		quote := target[0]
		end := strings.IndexByte(target[1:], quote)
		if end < 0 {
			return false
		}
		target = target[1 : end+1]
	} else {
		for i, ch := range target {
			if ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' || ch == ';' || ch == '|' || ch == '&' || ch == '>' {
				target = target[:i]
				break
			}
		}
	}
	target = strings.TrimSpace(target)
	return strings.EqualFold(target, "/dev/null") || strings.EqualFold(target, "nul") || strings.EqualFold(target, "$null")
}

// IsGitBranchMutation reports branch creation or mutation while retaining
// the established low-risk tier for branch listing/display commands.
func IsGitBranchMutation(command string) bool {
	trimmed := strings.TrimSpace(command)
	match := gitBranchCommandRe.FindStringIndex(trimmed)
	if match == nil {
		return false
	}
	rest := strings.TrimSpace(trimmed[match[1]:])
	if rest == "" {
		return false
	}
	tokens := strings.Fields(rest)
	allowOperands := false
	for i := 0; i < len(tokens); i++ {
		token := tokens[i]
		if token == "--" {
			return !allowOperands
		}
		if strings.HasPrefix(token, "--") {
			name, _, hasValue := strings.Cut(token, "=")
			switch name {
			case "--all", "--remotes", "--verbose", "--show-current", "--no-color", "--omit-empty":
				continue
			case "--list":
				allowOperands = true
				continue
			case "--format", "--column", "--sort", "--color":
				if hasValue {
					continue
				}
				if i+1 >= len(tokens) {
					return true
				}
				i++
				continue
			case "--contains", "--no-contains", "--merged", "--no-merged", "--points-at":
				if hasValue {
					continue
				}
				if i+1 >= len(tokens) {
					return true
				}
				i++
				continue
			default:
				return true
			}
		}
		if strings.HasPrefix(token, "-") {
			flags := strings.TrimPrefix(token, "-")
			if flags == "" {
				return true
			}
			readOnly := true
			for _, flag := range flags {
				if !strings.ContainsRune("aAvVrRlL", flag) {
					readOnly = false
					break
				}
			}
			if !readOnly {
				return true
			}
			if strings.ContainsAny(flags, "aAvVrRlL") {
				allowOperands = true
			}
			continue
		}
		if !allowOperands {
			return true
		}
	}
	return false
}

func extractCommand(question, context string) string {
	lines := append([]string{question}, strings.Split(context, "\n")...)
	candidates := make([]string, 0, 2)
	seen := make(map[string]struct{}, 2)
	addCandidate := func(value string) {
		value = compact(value)
		if value == "" {
			return
		}
		if _, ok := seen[value]; ok {
			return
		}
		seen[value] = struct{}{}
		candidates = append(candidates, value)
	}
	for i, rawLine := range lines {
		line := strings.TrimSpace(rawLine)
		if command, ok := commandFromLine(line); ok {
			addCandidate(command)
			continue
		}
		if !isCommandHeading(line) {
			continue
		}
		// Some providers render a heading on its own line and put the command
		// immediately below it. Skip repeated headings caused by a duplicated
		// context boundary, then consume the first actual command-looking line.
		for j := i + 1; j < len(lines); j++ {
			next := strings.TrimSpace(lines[j])
			if next == "" || isCommandHeading(next) {
				continue
			}
			if command, ok := commandFromLine(next); ok {
				addCandidate(command)
			} else {
				addCandidate(next)
			}
			break
		}
	}
	if len(candidates) == 0 {
		return compact(question)
	}
	return strings.Join(candidates, "\n")
}

// CommandFromLine reports whether a single prompt line names a command itself
// (e.g. "Run: git status"). Callers building an approval identity use it to
// tell a question that already carries the command from a fixed question whose
// command only appears in the prompt body.
func CommandFromLine(line string) (string, bool) {
	return commandFromLine(strings.TrimSpace(line))
}

func commandFromLine(line string) (string, bool) {
	for _, pattern := range commandPrefixes {
		if matches := pattern.FindStringSubmatch(line); len(matches) == 2 {
			return matches[1], true
		}
	}
	return "", false
}

func isCommandHeading(line string) bool {
	normalized := strings.ToLower(strings.TrimSpace(strings.TrimSuffix(line, ":")))
	switch normalized {
	case "bash command", "shell command":
		return true
	default:
		return false
	}
}

func extractPaths(value string) []string {
	matches := pathPattern.FindAllString(value, -1)
	paths := make([]string, 0, len(matches))
	seen := make(map[string]struct{}, len(matches))
	for _, path := range matches {
		path = strings.TrimRight(path, ".,;:)]}")
		if path == "" {
			continue
		}
		key := strings.ToLower(path)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		paths = append(paths, path)
		if len(paths) == 4 {
			break
		}
	}
	return paths
}

func compact(value string) string {
	return strings.TrimSpace(spacePattern.ReplaceAllString(value, " "))
}

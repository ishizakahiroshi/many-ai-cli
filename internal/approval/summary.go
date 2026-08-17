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
	// riskSegmentSplit breaks a command on shell connectors so each part is
	// classified independently (a low-risk first segment must not mask a
	// dangerous later one, e.g. "git status; rm -fr ~").
	riskSegmentSplit = regexp.MustCompile(`(?:&&|\|\||[;&\n|])`)
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
	lowPrefixes := []string{
		"cat ", "head ", "tail ", "ls", "dir", "pwd", "git status", "git diff",
		"git log", "git show", "git branch", "find ", "rg ", "grep ", "type ",
	}
	// Classify each connector-separated segment and take the worst tier. A
	// low-risk leading segment must not make the whole command low.
	sawLow, sawMid := false, false
	for _, seg := range riskSegmentSplit.Split(value, -1) {
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

func extractCommand(question, context string) string {
	lines := append([]string{question}, strings.Split(context, "\n")...)
	for _, line := range lines {
		line = strings.TrimSpace(line)
		for _, pattern := range commandPrefixes {
			if matches := pattern.FindStringSubmatch(line); len(matches) == 2 {
				return compact(matches[1])
			}
		}
	}
	return compact(question)
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

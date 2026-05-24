package hub

import (
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"any-ai-cli/internal/proto"
)

const approvalSourceGoVT = "go_vt"

type nativeApproval struct {
	Sig      string
	Kind     string
	Question string
	Context  string
	Options  []proto.ApprovalOption
}

var (
	numberedApprovalLineRe = regexp.MustCompile(`^\s*([>❯›❱])?\s*(\d{1,2})\.\s*(.+?)\s*$`)
	codexShortcutLineRe    = regexp.MustCompile(`^\s*([>❯›❱])?\s*(.+?)\s+\((y|p|n|esc|escape)\)\s*$`)
)

func detectNativeApproval(provider string, lines []string) *nativeApproval {
	recent := compactRecentLines(lines, 90)
	if len(recent) == 0 {
		return nil
	}
	opts, start, end := extractNativeApprovalOptions(provider, recent)
	if len(opts) == 0 {
		return nil
	}
	contextStart := maxInt(0, start-12)
	contextEnd := minInt(len(recent), end+6)
	contextLines := recent[contextStart:contextEnd]
	context := strings.Join(contextLines, "\n")
	question := nativeApprovalQuestion(contextLines, start-contextStart)
	if !nativeApprovalLooksValid(provider, contextLines, opts) {
		return nil
	}
	kind := "native"
	if provider == "codex" && approvalOptionsHaveSendText(opts) {
		kind = "native_codex_shortcut"
	}
	approval := &nativeApproval{
		Kind:     kind,
		Question: question,
		Context:  context,
		Options:  opts,
	}
	approval.Sig = nativeApprovalSig(provider, approval)
	return approval
}

func compactRecentLines(lines []string, limit int) []string {
	start := 0
	if limit > 0 && len(lines) > limit {
		start = len(lines) - limit
	}
	out := make([]string, 0, len(lines)-start)
	for _, line := range lines[start:] {
		out = append(out, strings.TrimRight(line, " "))
	}
	return out
}

func extractNativeApprovalOptions(provider string, lines []string) ([]proto.ApprovalOption, int, int) {
	type parsedLine struct {
		opt proto.ApprovalOption
		idx int
	}
	var parsed []parsedLine
	for i, line := range lines {
		if opt, ok := parseNativeApprovalOption(provider, line); ok {
			parsed = append(parsed, parsedLine{opt: opt, idx: i})
		}
	}
	if len(parsed) == 0 {
		return nil, -1, -1
	}

	bestStart, bestEnd := 0, 0
	curStart := 0
	for i := 1; i < len(parsed); i++ {
		if parsed[i].idx-parsed[i-1].idx > 4 {
			if i-curStart >= bestEnd-bestStart+1 {
				bestStart, bestEnd = curStart, i-1
			}
			curStart = i
		}
	}
	if len(parsed)-curStart >= bestEnd-bestStart+1 {
		bestStart, bestEnd = curStart, len(parsed)-1
	}

	cluster := parsed[bestStart : bestEnd+1]
	if len(cluster) < 2 {
		return nil, -1, -1
	}
	opts := make([]proto.ApprovalOption, 0, len(cluster))
	seen := make(map[string]struct{}, len(cluster))
	for _, item := range cluster {
		key := fmt.Sprintf("%d:%s:%s", item.opt.Num, item.opt.Label, item.opt.SendText)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		opts = append(opts, item.opt)
	}
	if len(opts) < 2 || len(opts) > 12 {
		return nil, -1, -1
	}
	if !approvalOptionsHaveCursor(opts) && !approvalOptionsHaveSendText(opts) {
		return nil, -1, -1
	}
	return opts, cluster[0].idx, cluster[len(cluster)-1].idx
}

func parseNativeApprovalOption(provider, line string) (proto.ApprovalOption, bool) {
	trimmed := strings.Trim(strings.TrimSpace(line), "│┃")
	trimmed = strings.TrimSpace(trimmed)
	if trimmed == "" {
		return proto.ApprovalOption{}, false
	}
	if m := numberedApprovalLineRe.FindStringSubmatch(trimmed); m != nil {
		n, _ := strconv.Atoi(m[2])
		label := cleanNativeApprovalLabel(m[3])
		if label == "" || n > 20 {
			return proto.ApprovalOption{}, false
		}
		opt := proto.ApprovalOption{
			Num:       n,
			Label:     label,
			IsCurrent: m[1] != "",
		}
		if sendText := codexShortcutSendText(label); provider == "codex" && sendText != "" {
			opt.SendText = sendText
			opt.PreserveOrder = true
		}
		return opt, true
	}
	if provider == "codex" {
		if m := codexShortcutLineRe.FindStringSubmatch(trimmed); m != nil {
			key := strings.ToLower(m[3])
			opt := proto.ApprovalOption{
				Num:           codexShortcutNum(key),
				Label:         cleanNativeApprovalLabel(fmt.Sprintf("%s (%s)", m[2], m[3])),
				IsCurrent:     m[1] != "",
				SendText:      codexShortcutSendText(key),
				PreserveOrder: true,
			}
			if opt.SendText != "" && opt.Label != "" {
				return opt, true
			}
		}
	}
	return proto.ApprovalOption{}, false
}

func cleanNativeApprovalLabel(label string) string {
	label = strings.TrimSpace(label)
	label = strings.Trim(label, "│┃")
	label = strings.Join(strings.Fields(label), " ")
	return strings.TrimSpace(label)
}

func codexShortcutNum(key string) int {
	switch strings.ToLower(key) {
	case "y":
		return 1
	case "p":
		return 2
	case "n":
		return 3
	case "esc", "escape":
		return 4
	default:
		return 0
	}
}

func codexShortcutSendText(label string) string {
	lower := strings.ToLower(strings.TrimSpace(label))
	if lower == "y" || strings.HasSuffix(lower, "(y)") {
		return "y"
	}
	if lower == "p" || strings.HasSuffix(lower, "(p)") {
		return "p"
	}
	if lower == "n" || strings.HasSuffix(lower, "(n)") {
		return "n"
	}
	if lower == "esc" || lower == "escape" || strings.HasSuffix(lower, "(esc)") || strings.HasSuffix(lower, "(escape)") {
		return "\x1b"
	}
	return ""
}

func nativeApprovalQuestion(contextLines []string, optionStart int) string {
	for i := optionStart - 1; i >= 0; i-- {
		line := strings.TrimSpace(contextLines[i])
		if line == "" {
			continue
		}
		return line
	}
	return ""
}

func nativeApprovalLooksValid(provider string, contextLines []string, opts []proto.ApprovalOption) bool {
	context := strings.ToLower(strings.Join(contextLines, "\n"))
	hasHint := strings.Contains(context, "approval") ||
		strings.Contains(context, "allow tool") ||
		strings.Contains(context, "allow this") ||
		strings.Contains(context, "requires approval") ||
		strings.Contains(context, "would you like to run") ||
		strings.Contains(context, "do you want to proceed") ||
		strings.Contains(context, "press enter to confirm") ||
		strings.Contains(context, "enter to select") ||
		strings.Contains(context, "esc to cancel")
	hasApprovalLabel := false
	for _, opt := range opts {
		lower := strings.ToLower(opt.Label)
		if strings.Contains(lower, "yes") ||
			strings.Contains(lower, "no") ||
			strings.Contains(lower, "allow") ||
			strings.Contains(lower, "deny") ||
			strings.Contains(lower, "proceed") ||
			strings.Contains(lower, "cancel") ||
			strings.Contains(lower, "don't ask") ||
			strings.Contains(lower, "dont ask") ||
			strings.Contains(lower, "(y)") ||
			strings.Contains(lower, "(n)") ||
			strings.Contains(lower, "(esc)") {
			hasApprovalLabel = true
			break
		}
	}
	if provider == "codex" && approvalOptionsHaveSendText(opts) {
		return hasHint
	}
	return hasHint && hasApprovalLabel
}

func approvalOptionsHaveCursor(opts []proto.ApprovalOption) bool {
	for _, opt := range opts {
		if opt.IsCurrent {
			return true
		}
	}
	return false
}

func approvalOptionsHaveSendText(opts []proto.ApprovalOption) bool {
	for _, opt := range opts {
		if opt.SendText != "" {
			return true
		}
	}
	return false
}

func nativeApprovalSig(provider string, approval *nativeApproval) string {
	var b strings.Builder
	b.WriteString(provider)
	b.WriteByte('\n')
	b.WriteString(approval.Kind)
	b.WriteByte('\n')
	b.WriteString(approval.Question)
	b.WriteByte('\n')
	b.WriteString(approval.Context)
	for _, opt := range approval.Options {
		b.WriteString(fmt.Sprintf("\n%d:%s:%s", opt.Num, opt.Label, opt.SendText))
	}
	sum := sha1.Sum([]byte(b.String()))
	return hex.EncodeToString(sum[:])[:16]
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}

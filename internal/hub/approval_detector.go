package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"any-ai-cli/internal/proto"
)

const approvalSourceGoVT = "go_vt"

// 承認検出のチューニング定数。
// approvalRecentLines: detectNativeApproval に渡す末尾行数の上限。
//
//	TailLines(vtTailLinesForApproval) と組み合わせて使う。
//	TailLines が 120 行取得し、そのうち末尾 90 行を有効な承認候補として扱う。
//	90 行は承認プロンプトが含まれる最大の行数（余白込み）の経験値。
//
// vtTailLinesForApproval: VT バッファから取り出す末尾行数（server.go と対応）。
const (
	approvalRecentLines       = 90
	vtTailLinesForApproval    = 120
	approvalMaxOptions        = 12
	approvalContextBefore     = 12
	approvalContextAfter      = 6
	approvalOptionGapLimit    = 4
	approvalOptionNumMaxLabel = 20
)

type nativeApproval struct {
	Sig      string
	Kind     string
	Question string
	Context  string
	Options  []proto.ApprovalOption
}

var (
	numberedApprovalLineRe = regexp.MustCompile(`^\s*([>❯›❱])?\s*(\d{1,2})\.\s*(.+?)\s*$`)
	shortcutApprovalLineRe = regexp.MustCompile(`^\s*([>❯›❱])?\s*(.+?)\s+\((y|p|n|!|#|\?|esc|escape)\)\s*$`)
	// cursor-agent の承認メニューはキー表記が多様（(y) / (tab) / (shift+tab) / (esc or n)）。
	// 末尾の (...) を緩く取り出し、cursorAgentKeyBinding で既知キーのみ採用する。
	cursorAgentShortcutLineRe = regexp.MustCompile(`^\s*([-*•>❯›❱])?\s*(.+?)\s+\(([^()]+)\)\s*$`)
)

func detectNativeApproval(provider string, lines []string) *nativeApproval {
	recent := compactRecentLines(lines, approvalRecentLines)
	if len(recent) == 0 {
		return nil
	}
	opts, start, end := extractNativeApprovalOptions(provider, recent)
	if len(opts) == 0 {
		return nil
	}
	contextStart := max(0, start-approvalContextBefore)
	contextEnd := min(len(recent), end+approvalContextAfter)
	contextLines := recent[contextStart:contextEnd]
	context := strings.Join(contextLines, "\n")
	question := nativeApprovalQuestion(contextLines, start-contextStart)
	if !nativeApprovalLooksValid(provider, contextLines, opts) {
		return nil
	}
	kind := "native"
	if provider == "codex" && approvalOptionsHaveSendText(opts) {
		kind = "native_codex_shortcut"
	} else if provider == "copilot" && approvalOptionsHaveSendText(opts) {
		kind = "native_copilot_shortcut"
	} else if provider == "cursor-agent" && approvalOptionsHaveSendText(opts) {
		kind = "native_cursor_agent_shortcut"
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
		if parsed[i].idx-parsed[i-1].idx > approvalOptionGapLimit {
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
	if len(opts) < 2 || len(opts) > approvalMaxOptions {
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
		if label == "" || n > approvalOptionNumMaxLabel {
			return proto.ApprovalOption{}, false
		}
		opt := proto.ApprovalOption{
			Num:       n,
			Label:     label,
			IsCurrent: m[1] != "",
		}
		if sendText := approvalShortcutSendText(provider, label); sendText != "" {
			opt.SendText = sendText
			opt.PreserveOrder = true
		}
		return opt, true
	}
	if provider == "cursor-agent" {
		return parseCursorAgentShortcutOption(trimmed)
	}
	if providerSupportsShortcutApproval(provider) {
		if m := shortcutApprovalLineRe.FindStringSubmatch(trimmed); m != nil {
			key := strings.ToLower(m[3])
			opt := proto.ApprovalOption{
				Num:           approvalShortcutNum(provider, key),
				Label:         cleanNativeApprovalLabel(fmt.Sprintf("%s (%s)", m[2], m[3])),
				IsCurrent:     m[1] != "",
				SendText:      approvalShortcutSendText(provider, key),
				PreserveOrder: true,
			}
			if opt.SendText != "" && opt.Label != "" {
				return opt, true
			}
		}
	}
	return proto.ApprovalOption{}, false
}

// parseCursorAgentShortcutOption は cursor-agent の承認メニュー 1 行をパースする。
// 実機 UI 例:
//
//	Run this command?
//	Not in allowlist: <command>
//	 - Run (once) (y)
//	    Add Shell(<cmd>) to allowlist? (tab)
//	    Auto-run everything (shift+tab)
//	    Skip (esc or n)
func parseCursorAgentShortcutOption(line string) (proto.ApprovalOption, bool) {
	m := cursorAgentShortcutLineRe.FindStringSubmatch(line)
	if m == nil {
		return proto.ApprovalOption{}, false
	}
	keyRaw := strings.TrimSpace(m[3])
	sendText, num := cursorAgentKeyBinding(keyRaw)
	if sendText == "" {
		return proto.ApprovalOption{}, false
	}
	label := cleanNativeApprovalLabel(fmt.Sprintf("%s (%s)", m[2], keyRaw))
	if label == "" {
		return proto.ApprovalOption{}, false
	}
	return proto.ApprovalOption{
		Num:           num,
		Label:         label,
		IsCurrent:     isCursorAgentCurrentMarker(m[1]),
		SendText:      sendText,
		PreserveOrder: true,
	}, true
}

// cursorAgentKeyBinding は cursor-agent のキー表記を PTY 送信バイトと表示番号に変換する。
// 既知キー以外は sendText="" を返し、呼び出し側で承認オプションから除外させる
// （末尾が "(...)" の無関係な行を誤って拾わないためのフィルタを兼ねる）。
func cursorAgentKeyBinding(key string) (sendText string, num int) {
	switch strings.ToLower(strings.TrimSpace(key)) {
	case "y":
		return "y", 1
	case "tab":
		return "\t", 2
	case "shift+tab", "shift + tab":
		return "\x1b[Z", 3
	case "esc or n", "n or esc", "esc", "escape":
		return "\x1b", 4
	case "n":
		return "n", 4
	}
	return "", 0
}

func isCursorAgentCurrentMarker(prefix string) bool {
	switch prefix {
	case ">", "❯", "›", "❱", "-", "*", "•":
		return true
	}
	return false
}

func cleanNativeApprovalLabel(label string) string {
	label = strings.TrimSpace(label)
	label = strings.Trim(label, "│┃")
	label = strings.Join(strings.Fields(label), " ")
	return strings.TrimSpace(label)
}

func providerSupportsShortcutApproval(provider string) bool {
	return provider == "codex" || provider == "copilot" || provider == "cursor-agent"
}

func approvalShortcutNum(provider, key string) int {
	switch strings.ToLower(key) {
	case "y":
		return 1
	case "p":
		if provider != "codex" {
			return 0
		}
		return 2
	case "n":
		if provider == "copilot" || provider == "cursor-agent" {
			return 2
		}
		return 3
	case "!":
		if provider == "copilot" || provider == "cursor-agent" {
			return 3
		}
	case "#":
		if provider == "copilot" || provider == "cursor-agent" {
			return 4
		}
	case "?":
		if provider == "copilot" || provider == "cursor-agent" {
			return 5
		}
	case "esc", "escape":
		if provider == "copilot" || provider == "cursor-agent" {
			return 6
		}
		return 4
	}
	return 0
}

func approvalShortcutSendText(provider, label string) string {
	if !providerSupportsShortcutApproval(provider) {
		return ""
	}
	lower := strings.ToLower(strings.TrimSpace(label))
	if lower == "y" || strings.HasSuffix(lower, "(y)") {
		return "y"
	}
	if provider == "codex" && (lower == "p" || strings.HasSuffix(lower, "(p)")) {
		return "p"
	}
	if lower == "n" || strings.HasSuffix(lower, "(n)") {
		return "n"
	}
	if (provider == "copilot" || provider == "cursor-agent") && (lower == "!" || strings.HasSuffix(lower, "(!)")) {
		return "!"
	}
	if (provider == "copilot" || provider == "cursor-agent") && (lower == "#" || strings.HasSuffix(lower, "(#)")) {
		return "#"
	}
	if (provider == "copilot" || provider == "cursor-agent") && (lower == "?" || strings.HasSuffix(lower, "(?)")) {
		return "?"
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
		strings.Contains(context, "requires permission") ||
		strings.Contains(context, "requires confirmation") ||
		strings.Contains(context, "permission required") ||
		strings.Contains(context, "permissions required") ||
		strings.Contains(context, "user confirmation") ||
		strings.Contains(context, "would you like to run") ||
		strings.Contains(context, "do you want to proceed") ||
		strings.Contains(context, "press enter to confirm") ||
		strings.Contains(context, "enter to select") ||
		strings.Contains(context, "esc to cancel")
	// cursor-agent 実機 UI 特有のヒント（"Run this command?" / "Not in allowlist:" /
	// "Add ... to allowlist?" / "Auto-run everything"）を追加で許容する。
	if provider == "cursor-agent" && !hasHint {
		hasHint = strings.Contains(context, "allowlist") ||
			strings.Contains(context, "run this command") ||
			strings.Contains(context, "auto-run")
	}
	hasApprovalLabel := false
	for _, opt := range opts {
		lower := strings.ToLower(opt.Label)
		if strings.Contains(lower, "yes") ||
			strings.Contains(lower, "no") ||
			strings.Contains(lower, "allow") ||
			strings.Contains(lower, "deny") ||
			strings.Contains(lower, "once") ||
			strings.Contains(lower, "always") ||
			strings.Contains(lower, "all similar") ||
			strings.Contains(lower, "details") ||
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
	if providerSupportsShortcutApproval(provider) && approvalOptionsHaveSendText(opts) {
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
	sum := sha256.Sum256([]byte(b.String()))
	return hex.EncodeToString(sum[:])[:16]
}

package hub

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"many-ai-cli/internal/approval"
	"many-ai-cli/internal/proto"
)

const approvalSourceGoVT = "go_vt"

// nativeApprovalJaTokens は日本語ネイティブ承認プロンプトで出現するヒント語。
// Go VT 検出器（server.go の nativeApprovalTriggerTokens と
// nativeApprovalLooksValid の hasHint）で共通参照する single source。
// TS 側（approval.ts の matchProviderApprovalTrigger / common パターン JSON）とは
// 役割が異なるため別管理だが、意味的に重複しない語を選んでいる。
// 二重発火の回避: TS 側スキャンは xterm.js バッファ全体を対象とし、Go 側は
// PTY チャンクをトリガーにして VT テールを再スキャンする別経路。
// 同一 sig が検出されても Hub は sig 一致で重複送信をスキップするため二重発火しない。
var nativeApprovalJaTokens = []string{
	"許可",       // 「この操作を許可しますか」等
	"承認",       // 「承認しますか」等
	"続行",       // 「続行しますか」等
	"実行しますか",   // 「コマンドを実行しますか」等
	"よろしいですか",  // 「よろしいですか？」等
	"確認してください", // 「操作を確認してください」等
}

// 承認検出のチューニング定数。
// approvalRecentLines: detectNativeApproval に渡す末尾行数の上限。
//
//	TailLines(vtTailLinesForApproval) と組み合わせて使う。
//	TailLines が 120 行取得し、そのうち末尾 90 行を有効な承認候補として扱う。
//	90 行は承認プロンプトが含まれる最大の行数（余白込み）の経験値。
//
// vtTailLinesForApproval: ネイティブ承認検出向けに VT 画面から取り出す末尾行数。
//
// vtTailLinesForMarker: 承認マーカー抽出向け。scrollback 込みで取り出す末尾行数。
// 画面高（~30）を超える複数質問ブロック（Grok 等）を拾うため 300 行を確保する。
const (
	approvalRecentLines       = 90
	vtTailLinesForApproval    = 120
	vtTailLinesForMarker      = 300
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
	Summary  proto.ApprovalSummary
}

var (
	numberedApprovalLineRe = regexp.MustCompile(`^\s*([>❯›❱])?\s*(\d{1,2})\.\s*(.+?)\s*$`)
	shortcutApprovalLineRe = regexp.MustCompile(`^\s*([>❯›❱])?\s*(.+?)\s+\((y|p|n|!|#|\?|esc|escape)\)\s*$`)
	// cursor-agent の承認メニューはキー表記が多様（(y) / (tab) / (shift+tab) / (esc or n)）。
	// 末尾の (...) を緩く取り出し、cursorAgentKeyBinding で既知キーのみ採用する。
	cursorAgentShortcutLineRe = regexp.MustCompile(`^\s*([-*•>❯›❱])?\s*(.+?)\s+\(([^()]+)\)\s*$`)
	// grokRadioApprovalLineRe は Grok Build のツール許可カード。
	// 実機 PTY（2026-08-20 セッション 8 / 08-19 s9,s10）:
	//   1 (•) Yes, and don't ask again for anything (always-approve mode)
	//   2 (○) Yes, proceed
	//   3 (○) No, reject (type to add feedback)
	// 番号の直後はピリオドではなくラジオ印。選択中は • / ● / * / - 、未選択は ○。
	grokRadioApprovalLineRe = regexp.MustCompile(`^\s*(\d{1,2})\s+\(([•●○*\-])\)\s+(.+?)\s*$`)
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
	before, after := approvalContextBefore, approvalContextAfter
	if provider == "opencode" {
		// OpenCode は start/end がダイアログ本体（"Permission required" 〜 ボタン行）を
		// 指すため、周囲の余白を取らない。余白を取ると承認とは無関係な行
		//（ストリーミング中の応答・スピナー・経過秒）が context に入り、
		// 画面が 1 文字変わるたびに Sig が変わって承認が再配信される
		// （= Web の action-bar が毎フレーム作り直され、点滅してクリックを取りこぼす）。
		before, after = 0, 1
	}
	contextStart := max(0, start-before)
	contextEnd := min(len(recent), end+after)
	contextLines := recent[contextStart:contextEnd]
	context := strings.Join(contextLines, "\n")
	question := nativeApprovalQuestion(contextLines, start-contextStart)
	hintLines := contextLines
	if provider == "opencode" {
		// 選択肢がダイアログ範囲に内包されるため nativeApprovalQuestion では
		// 質問行が取れない（常に空文字になる）。専用に取り出す。
		if openCodeIsAlwaysAllowDialog(contextLines) {
			question = openCodeAlwaysAllowQuestion(contextLines)
		} else {
			question = openCodeApprovalQuestion(contextLines)
		}
		// ヒント語・モデルセレクタの判定は context ではなく画面全体（recent）を見る。
		// context を絞ったことで判定材料まで痩せると、承認の取りこぼし（"permission required"
		// がボタン行より前にある場合）や /model セレクタの誤検出抑制の失効を招くため。
		hintLines = recent
		if looksLikeOpenCodeModelSelector(recent) {
			return nil
		}
	}
	if !nativeApprovalLooksValid(provider, hintLines, opts) {
		return nil
	}
	// AI が自発的に出す Claude AskUserQuestion ピッカー（末尾に "Type something" /
	// "Chat about this" の自由入力肢を持つ arrow 駆動 UI）は webify しない。
	// 再描画される VT をスクレイプして Web ボタン化すると選択肢番号がズレて誤選択を
	// 招くため（approval-rules.md version 10 で AI を [MANY-AI-CLI] マーカーへ誘導済み）。
	// 万一 AI が出しても Web バーは出さず、ユーザーは端末で直接 ↑↓/Enter 操作する。
	if looksLikeNativeAskUserQuestion(opts) {
		return nil
	}
	kind := "native"
	if provider == "codex" && approvalOptionsHaveSendText(opts) {
		kind = "native_codex_shortcut"
	} else if provider == "copilot" && approvalOptionsHaveSendText(opts) {
		kind = "native_copilot_shortcut"
	} else if provider == "cursor-agent" && approvalOptionsHaveSendText(opts) {
		kind = "native_cursor_agent_shortcut"
	} else if provider == "opencode" && approvalOptionsHaveSendText(opts) {
		kind = "native_opencode_shortcut"
	}
	approval := &nativeApproval{
		Kind:     kind,
		Question: question,
		Context:  context,
		Options:  opts,
		Summary:  approval.Summarize(question, context),
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

// extractOpenCodeApprovalOptions は OpenCode の水平 3 ボタン UI
// (Allow once / Allow always / Reject) を検出し合成オプションを返す。
// "allow once" の文言が PTY バッファに現れた時点でオプションを確定する
// (初期フォーカスは常に "Allow once"。矢印 + Enter で移動・確定)。
//
// 併せてダイアログ本体の行範囲 [start, end] を返す。end はボタン行、start は
// その直近上にある "Permission required" 行（無ければボタン行と同じ）。
// この範囲だけを context にすることで、承認の同一性が画面全体のスナップショットに
// 依存しなくなる（点滅・クリック取りこぼしの根本）。
func extractOpenCodeApprovalOptions(lines []string) ([]proto.ApprovalOption, int, int) {
	// 画面に解決済みの古いダイアログが残っていることがあるため、最後（＝最新）の
	// ボタン行を採用する。先頭からの一致を採ると、下にある本物の承認要求ではなく
	// 上に残った古いダイアログで Sig が固定され、新しい承認を取り違える。
	for i := len(lines) - 1; i >= 0; i-- {
		if !strings.Contains(strings.ToLower(lines[i]), "allow once") {
			continue
		}
		opts := []proto.ApprovalOption{
			{Num: 1, Label: "Allow once", SendText: "\r", IsCurrent: true, PreserveOrder: true},
			{Num: 2, Label: "Allow always", SendText: "\x1b[C\r", PreserveOrder: true},
			{Num: 3, Label: "Reject", SendText: "\x1b[C\x1b[C\r", PreserveOrder: true},
		}
		return opts, openCodeDialogStart(lines, i), i
	}
	return nil, -1, -1
}

// openCodeDialogStart はボタン行 buttonIdx から上へ遡り、ダイアログの見出し
// （"Permission required"）の行番号を返す。遡る幅はダイアログ本体に収まる範囲に限る。
// 見出しが見つからないときはボタン行ではなく遡り上限を返す: context がボタン行だけに
// なると全ての承認が同一 Sig になり、別々の承認要求が「回答済み」で握り潰されるため。
func openCodeDialogStart(lines []string, buttonIdx int) int {
	limit := max(0, buttonIdx-approvalContextBefore)
	for i := buttonIdx; i >= limit; i-- {
		if strings.Contains(strings.ToLower(lines[i]), "permission required") {
			return i
		}
	}
	return limit
}

// openCodeApprovalQuestion はダイアログ本体から「何を許可しようとしているか」の 1 行を返す。
// 見出し行（"Permission required"）とボタン行を除いた最初の非空行を採用する
// （実機 UI 例: "Permission required" → "Read CLAUDE.md" → "Path: CLAUDE.md" → ボタン行）。
// nativeApprovalQuestion は「選択肢クラスタより前の行」を見る作りで、選択肢が
// ダイアログ範囲に内包される OpenCode では空文字になるため専用に取り出す。
func openCodeApprovalQuestion(contextLines []string) string {
	for _, line := range contextLines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		lower := strings.ToLower(trimmed)
		if strings.Contains(lower, "permission required") || strings.Contains(lower, "allow once") {
			continue
		}
		return trimmed
	}
	return ""
}

// extractOpenCodeAlwaysAllowOptions は OpenCode の 2 段目ダイアログ
// (Allow always を選んだ直後に出る「Always allow」確認) を検出し合成オプションを返す。
// 実機 UI（PTY 生ログ実測）:
//
//	△ Always allow
//	This will allow the following patterns until OpenCode is restarted
//	- opencode *
//	Confirm  Cancel                              ⇆ select  enter confirm
//
// 初期フォーカスは "Confirm"（1 段目と同じく矢印 + Enter で移動・確定）。
// この 2 段目は "allow once" を含まないため extractOpenCodeApprovalOptions では
// 拾えず、Web の action-bar が出ないまま端末での Enter を強いていた。
//
// 併せてダイアログ本体の行範囲 [start, end] を返す。end はボタン行、start は
// 見出し行（"Always allow"）。見出しが遡り範囲に無い行は、応答本文に現れた
// "confirm" / "cancel" の誤検出とみなして採用しない。
func extractOpenCodeAlwaysAllowOptions(lines []string) ([]proto.ApprovalOption, int, int) {
	for i := len(lines) - 1; i >= 0; i-- {
		if !isOpenCodeConfirmButtonLine(lines[i]) {
			continue
		}
		start := openCodeAlwaysAllowStart(lines, i)
		if start < 0 {
			continue
		}
		opts := []proto.ApprovalOption{
			{Num: 1, Label: "Confirm", SendText: "\r", IsCurrent: true, PreserveOrder: true},
			{Num: 2, Label: "Cancel", SendText: "\x1b[C\r", PreserveOrder: true},
		}
		return opts, start, i
	}
	return nil, -1, -1
}

// isOpenCodeConfirmButtonLine は "Confirm" と "Cancel" が同居する水平ボタン行か判定する。
// フッターのキーヒント行（"⇆ select  enter confirm"）は "cancel" を含まないため除外される。
func isOpenCodeConfirmButtonLine(line string) bool {
	lower := strings.ToLower(line)
	return strings.Contains(lower, "confirm") && strings.Contains(lower, "cancel")
}

// openCodeAlwaysAllowStart はボタン行 buttonIdx から上へ遡り、見出し（"Always allow"）
// の行番号を返す。見つからなければ -1（＝このボタン行はダイアログではない）。
func openCodeAlwaysAllowStart(lines []string, buttonIdx int) int {
	limit := max(0, buttonIdx-approvalContextBefore)
	for i := buttonIdx; i >= limit; i-- {
		if strings.Contains(strings.ToLower(lines[i]), "always allow") {
			return i
		}
	}
	return -1
}

// openCodeIsAlwaysAllowDialog は context の見出し行が 2 段目ダイアログか判定する。
// context は before=0 で切り出しているため contextLines[0] が見出し行になる。
func openCodeIsAlwaysAllowDialog(contextLines []string) bool {
	for _, line := range contextLines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		return strings.Contains(strings.ToLower(trimmed), "always allow")
	}
	return false
}

// openCodeAlwaysAllowQuestion は 2 段目ダイアログから「何を恒久的に許可するのか」を返す。
// 許可パターン行（"- opencode *" 等）があればそれを優先し、無ければ説明文
// （"This will allow webfetch until OpenCode is restarted." のような 1 行完結型）を返す。
// パターンは何を許可したかの実体そのものなので、これを Sig と表示の軸にする。
func openCodeAlwaysAllowQuestion(contextLines []string) string {
	fallback := ""
	for _, line := range contextLines {
		trimmed := cleanNativeApprovalLabel(line)
		if trimmed == "" {
			continue
		}
		lower := strings.ToLower(trimmed)
		if strings.Contains(lower, "always allow") || isOpenCodeConfirmButtonLine(lower) {
			continue
		}
		if pattern := strings.TrimSpace(strings.TrimPrefix(trimmed, "-")); strings.HasPrefix(trimmed, "-") && pattern != "" {
			return pattern
		}
		if fallback == "" {
			fallback = trimmed
		}
	}
	return fallback
}

func looksLikeOpenCodeModelSelector(lines []string) bool {
	context := strings.ToLower(strings.Join(lines, "\n"))
	if !strings.Contains(context, "select model") {
		return false
	}
	return strings.Contains(context, "connect provider") ||
		strings.Contains(context, "favorite") ||
		strings.Contains(context, "opencode zen") ||
		strings.Contains(context, "ollama (local)") ||
		strings.Contains(context, "recent")
}

// extractOpenCodeOptions は OpenCode の 2 種類の承認ダイアログを検出する。
//   - 1 段目: "Permission required"（Allow once / Allow always / Reject）
//   - 2 段目: "Always allow"（Confirm / Cancel）— Allow always を選んだ後の確認
//
// 画面には解決済みの 1 段目が残ったまま 2 段目が描かれることがあるため、
// より下（＝新しい）ダイアログを採用する。
func extractOpenCodeOptions(lines []string) ([]proto.ApprovalOption, int, int) {
	permOpts, permStart, permEnd := extractOpenCodeApprovalOptions(lines)
	confirmOpts, confirmStart, confirmEnd := extractOpenCodeAlwaysAllowOptions(lines)
	if len(confirmOpts) >= 2 && confirmEnd > permEnd {
		return confirmOpts, confirmStart, confirmEnd
	}
	if len(permOpts) >= 2 {
		return permOpts, permStart, permEnd
	}
	return nil, -1, -1
}

func extractNativeApprovalOptions(provider string, lines []string) ([]proto.ApprovalOption, int, int) {
	if provider == "opencode" {
		return extractOpenCodeOptions(lines)
	}
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
	if provider == "grok" {
		if opt, ok := parseGrokRadioApprovalOption(trimmed); ok {
			return opt, true
		}
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

func parseGrokRadioApprovalOption(line string) (proto.ApprovalOption, bool) {
	m := grokRadioApprovalLineRe.FindStringSubmatch(line)
	if m == nil {
		return proto.ApprovalOption{}, false
	}
	n, _ := strconv.Atoi(m[1])
	label := cleanNativeApprovalLabel(m[3])
	if label == "" || n > approvalOptionNumMaxLabel {
		return proto.ApprovalOption{}, false
	}
	return proto.ApprovalOption{
		Num:       n,
		Label:     label,
		IsCurrent: m[2] != "○",
	}, true
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

// looksLikeNativeAskUserQuestion は、選択肢ラベルに Claude AskUserQuestion 特有の
// 自由入力肢（"Type something" / "Chat about this"）が含まれるかを判定する。
// 標準のツール許可プロンプト（Yes / Yes, and / No）はこれらを含まないため誤検出しない。
func looksLikeNativeAskUserQuestion(opts []proto.ApprovalOption) bool {
	for _, o := range opts {
		l := strings.ToLower(strings.TrimSpace(o.Label))
		if strings.HasPrefix(l, "type something") || strings.HasPrefix(l, "chat about") {
			return true
		}
	}
	return false
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
	// Note: context は小文字化しているが nativeApprovalJaTokens は日本語（大文字化不要）なので
	// 元の文字列でも検索する必要がある。rawContext を別途用意する。
	rawContext := strings.Join(contextLines, "\n")
	context := strings.ToLower(rawContext)
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
	// 日本語ネイティブ承認プロンプトのヒント語（nativeApprovalJaTokens と共通）。
	if !hasHint {
		for _, tok := range nativeApprovalJaTokens {
			if strings.Contains(rawContext, tok) {
				hasHint = true
				break
			}
		}
	}
	// cursor-agent 実機 UI 特有のヒント（"Run this command?" / "Not in allowlist:" /
	// "Add ... to allowlist?" / "Auto-run everything"）を追加で許容する。
	if provider == "cursor-agent" && !hasHint {
		hasHint = strings.Contains(context, "allowlist") ||
			strings.Contains(context, "run this command") ||
			strings.Contains(context, "auto-run")
	}
	// OpenCode の 2 段目ダイアログ（Allow always の確認）は "permission required" を
	// 含まないため、専用の見出し・説明文をヒントとして許容する。
	if provider == "opencode" && !hasHint {
		hasHint = strings.Contains(context, "always allow") ||
			strings.Contains(context, "until opencode is restarted")
	}
	// Grok Build のツール許可カード。ラベルの always-approve は "approval" 部分一致でも
	// 拾えるが、フッターだけが context に残った再描画でも落とさない。
	if provider == "grok" && !hasHint {
		hasHint = strings.Contains(context, "tab:next option") ||
			strings.Contains(context, "always-approve") ||
			strings.Contains(context, "type to add feedback")
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
	// Claude / Grok の /model 等カーソル駆動セレクタ型ダイアログは Hub の action-bar に出さない。
	// 全 AI で UX を統一する方針（codex / opencode の /model も isModelSelectorContext で
	// 抑制済み）。承認語ラベルを含まない選択肢は承認ではないため、ここでも false を返し
	// 端末直操作へフォールバックさせる（過去には isSelectorDialog 分岐で許容していたが、
	// 「ポップアップ不要・全 AI 統一」のユーザー要望で削除）。
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
	return approvalCandidateKeyWithContext(provider, approval.Kind, approval.Question, approval.Context, approval.Options)
}

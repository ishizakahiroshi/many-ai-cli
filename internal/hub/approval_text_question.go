package hub

import (
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionstore"
)

// approval_text_question.go: 「次のユーザーの発話で答える文章の質問」を Hub で検出する。
//
// 承認マーカー（開始・終了マーカーで囲んだブロック）のほかに、以前はブラウザだけが端末の文字から拾っていた
// 文章の質問が 3 種類ある（親 plan の D1。2026-09-23 に 4 種とも Hub へ移すと決めた）。
//
//   - マーカー無しの Yes/No（行末が (Y:1/N:0) の質問）
//   - 順次質問（Q1: / 問1: などの見出しに番号付きの選択肢が続く質問が 2 つ以上）
//   - 旧形式の選択（「どちらで進めますか」の後に選択肢と N. User specifies が続く）
//
// どれも CLI の承認画面ではなく AI が書いた文章なので、答えは次のユーザーの発話になる。
// 記録の閉じ方も承認マーカーと同じにする（Origin はマーカーと同じ approvalRecordOriginMarker。
// approval_record.go 冒頭）。検出の供給元もマーカーと同じで、claude / codex はトランスクリプト、
// それ以外は端末ミラー（approval_marker_transcript.go の 1 セッション 1 供給元のルール）。
// ただし開くのはマーカーと違い、出力が落ち着いてから（下の「出力が落ち着いてから開く」節）。
//
// 判定は以前ブラウザ（web/src/app/approval-parser.ts）にあった関数を写したもので、2026-09-23 に
// ブラウザ側の検出を撤去した後は Hub だけが持つ。入力と期待値は approval_text_question_test.go。
//
// 記録の Block には、画面のパーサ（Hub のブロック用）がそのまま読める形に整えた本文を入れる
// （Yes/No と旧形式の選択はマーカーの中身と同じ形、順次質問は「見出し: 質問」と選択肢の行）。
// 画面はこの Block を描く（web/src/app/approval-store.ts）。順次質問のパーサは両方にあるので、
// 片方を直すときはもう片方も直す。

const (
	approvalKindMarker           = "marker"
	approvalKindPlainYesNo       = "plain_yes_no"
	approvalKindSequentialChoice = "sequential_choice"
	approvalKindHubChoice        = "hub_choice"
)

// textQuestion は次のユーザーの発話で答える質問 1 件。承認マーカーもこの形で記録を開く。
type textQuestion struct {
	Kind     string
	Block    string
	Sig      string
	Question string
	Options  []proto.ApprovalOption
}

// candidateIdentity は承認の同一性（approval_identity.go）。承認マーカーはブロックから
// 意味のある部分だけを取る既存の作り方、それ以外は質問文と選択肢番号から作る。
func (q *textQuestion) candidateIdentity(provider string) struct{ key, shape string } {
	if q.Kind == approvalKindMarker {
		return approvalMarkerCandidateIdentity(provider, q.Block)
	}
	return struct{ key, shape string }{
		key:   approvalCandidateKey(provider, q.Kind, q.Question, q.Options),
		shape: approvalCandidateShape(provider, q.Kind, q.Question, q.Options),
	}
}

func newTextQuestion(kind, block, question string, options []proto.ApprovalOption) *textQuestion {
	return &textQuestion{
		Kind:     kind,
		Block:    block,
		Sig:      approvalMarkerSignature(block),
		Question: question,
		Options:  options,
	}
}

// detectTextQuestion はマーカー無しの文章の質問を 1 件返す。順番は以前のブラウザの走査と
// 同じ（Yes/No → 順次質問 → 旧形式の選択）。
// 承認マーカーはこれより先に呼び出し側で見る。
func detectTextQuestion(lines []string) *textQuestion {
	// 端末ミラーの画面は出力の下に空行が並ぶ。以前のブラウザが見ていたのは出力の流れの末尾で、
	// この空行を含まない。含めたまま「末尾 20 行」を取ると、画面の上の方に出た質問が窓から外れる。
	for len(lines) > 0 && strings.TrimSpace(lines[len(lines)-1]) == "" {
		lines = lines[:len(lines)-1]
	}
	if q := extractPlainYesNoQuestion(lines); q != nil {
		return q
	}
	if q := extractSequentialChoiceQuestion(lines); q != nil {
		return q
	}
	return extractHubChoiceQuestion(lines)
}

// ---- マーカー無しの Yes/No（以前の approval-parser.ts の extractPlainYesNoApproval を写したもの）----

var (
	yesNoApprovalMarkerRe      = regexp.MustCompile(`(?i)[（(]\s*[YＹ]\s*[:：]\s*1\s*[/／]\s*[NＮ]\s*[:：]\s*0\s*[）)]`)
	approvalMarkerTokenRe      = regexp.MustCompile(regexp.QuoteMeta(approvalMarkerOpen) + "|" + regexp.QuoteMeta(approvalMarkerClose))
	placeholderYesNoQuestionRe = regexp.MustCompile(`(?i)^question\s*\d*\s*[?？]$`)
	questionMarkEndRe          = regexp.MustCompile(`[?？]\s*$`)
)

func hasApprovalMarkerToken(text string) bool {
	return strings.Contains(text, approvalMarkerOpen) || strings.Contains(text, approvalMarkerClose)
}

func yesNoQuestionText(text string) string {
	matches := yesNoApprovalMarkerRe.FindAllStringIndex(text, -1)
	if len(matches) == 0 {
		return ""
	}
	before := text[:matches[len(matches)-1][0]]
	before = approvalMarkerTokenRe.ReplaceAllString(before, "")
	return strings.Join(strings.Fields(before), " ")
}

func looksLikeYesNoQuestion(text string) bool {
	if !yesNoApprovalMarkerRe.MatchString(text) {
		return false
	}
	before := yesNoQuestionText(text)
	if placeholderYesNoQuestionRe.MatchString(before) {
		return false
	}
	if questionMarkEndRe.MatchString(strings.TrimSpace(before)) {
		return true
	}
	return strings.ContainsAny(lastRunes(before, 120), "?？")
}

func lastRunes(s string, n int) string {
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	runes := []rune(s)
	return string(runes[len(runes)-n:])
}

func plainYesNoOptions() []proto.ApprovalOption {
	return []proto.ApprovalOption{
		{Num: 1, Label: "Yes (1)", IsCurrent: true, PreserveOrder: true},
		{Num: 0, Label: "No (0)", PreserveOrder: true},
	}
}

func newPlainYesNoQuestion(text string) *textQuestion {
	question := yesNoQuestionText(text)
	block := strings.Join(strings.Fields(approvalMarkerTokenRe.ReplaceAllString(text, "")), " ")
	return newTextQuestion(approvalKindPlainYesNo, block, question, plainYesNoOptions())
}

// extractPlainYesNoQuestion は末尾 20 行から、マーカーの外にある (Y:1/N:0) の質問を取り出す。
func extractPlainYesNoQuestion(lines []string) *textQuestion {
	searchStart := max(0, len(lines)-20)
	recent := make([]string, 0, len(lines)-searchStart)
	for _, line := range lines[searchStart:] {
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			recent = append(recent, trimmed)
		}
	}
	for i := len(lines) - 1; i >= searchStart; i-- {
		line := strings.TrimSpace(lines[i])
		if line == "" || hasApprovalMarkerToken(line) {
			continue
		}
		if looksLikeYesNoQuestion(line) {
			return newPlainYesNoQuestion(line)
		}
	}
	// 行の途中で折り返された質問（「…しますか？ (Y:1/」「N:0)」）は連結して見る。
	recentText := strings.Join(recent, "\n")
	if !hasApprovalMarkerToken(recentText) && looksLikeYesNoQuestion(recentText) {
		return newPlainYesNoQuestion(recentText)
	}
	return nil
}

// ---- 順次質問（approval-parser.ts の extractSequentialChoicePrompts）----

var (
	sequentialQuestionHeaderRe = regexp.MustCompile(`(?i)^\s*([A-Z]{1,3}\d{1,3}|Q\d{1,3}|問\d{1,3})\s*[:：]\s*(.+?)\s*$`)
	sequentialOptionRe         = regexp.MustCompile(`^\s*(\d{1,2})\.\s*(.+?)\s*$`)
	sequentialUserSpecifiesRe  = regexp.MustCompile(`(?i)^\s*N\.\s*(User specifies|その他指定)`)
	leadingTwoSpacesRe         = regexp.MustCompile(`^\s{2,}`)
)

type sequentialPrompt struct {
	Key      string
	Question string
	Options  []proto.ApprovalOption
}

func extractSequentialChoicePrompts(lines []string) []sequentialPrompt {
	var prompts []sequentialPrompt
	var current *sequentialPrompt
	start := max(0, len(lines)-80)
	for _, raw := range lines[start:] {
		rawLine := strings.TrimRightFunc(raw, unicode.IsSpace)
		line := strings.TrimSpace(rawLine)
		if line == "" {
			continue
		}
		if hasApprovalMarkerToken(line) {
			return nil
		}
		if m := sequentialQuestionHeaderRe.FindStringSubmatch(line); m != nil {
			if current != nil && len(current.Options) >= 2 {
				prompts = append(prompts, *current)
			}
			current = &sequentialPrompt{Key: strings.TrimSpace(m[1]), Question: strings.TrimSpace(m[2])}
			continue
		}
		if current == nil {
			continue
		}
		if m := sequentialOptionRe.FindStringSubmatch(line); m != nil {
			n, _ := strconv.Atoi(m[1])
			current.Options = append(current.Options, proto.ApprovalOption{
				Num:       n,
				Label:     strings.TrimSpace(m[2]),
				IsCurrent: len(current.Options) == 0,
			})
			continue
		}
		if sequentialUserSpecifiesRe.MatchString(line) {
			continue
		}
		if len(current.Options) > 0 && !leadingTwoSpacesRe.MatchString(rawLine) {
			if len(current.Options) >= 2 {
				prompts = append(prompts, *current)
			}
			current = nil
		}
	}
	if current != nil && len(current.Options) >= 2 {
		prompts = append(prompts, *current)
	}

	unique := make([]sequentialPrompt, 0, len(prompts))
	seen := map[string]bool{}
	for _, prompt := range prompts {
		key := prompt.Key + ":" + prompt.Question
		if seen[key] {
			continue
		}
		seen[key] = true
		sort.SliceStable(prompt.Options, func(i, j int) bool { return prompt.Options[i].Num < prompt.Options[j].Num })
		unique = append(unique, prompt)
	}
	if len(unique) < 2 {
		return nil
	}
	return unique
}

func extractSequentialChoiceQuestion(lines []string) *textQuestion {
	prompts := extractSequentialChoicePrompts(lines)
	if prompts == nil {
		return nil
	}
	var block, questions []string
	var options []proto.ApprovalOption
	for _, prompt := range prompts {
		header := prompt.Key + ": " + prompt.Question
		questions = append(questions, header)
		block = append(block, header)
		for _, opt := range prompt.Options {
			block = append(block, "  "+strconv.Itoa(opt.Num)+". "+opt.Label)
			options = append(options, proto.ApprovalOption{Num: opt.Num, Label: opt.Label})
		}
	}
	return newTextQuestion(approvalKindSequentialChoice, strings.Join(block, "\n"), strings.Join(questions, "\n"), options)
}

// ---- 旧形式の選択（以前の approval-parser.ts の extractApprovalOptions + isHubChoicePrompt を写したもの）----

var (
	userSpecifiesRe       = regexp.MustCompile(`(?i)user specifies|その他指定`)
	hubChoiceQuestionRe   = regexp.MustCompile(`(?i)どれで進めますか|どれで進める|どちらで進め|どの選択肢|選択してください|how would you like to proceed|which option`)
	recommendedChoiceRe   = regexp.MustCompile(`(?i)\(recommended\)|（recommended）|推奨`)
	fallbackCursorOptRe   = regexp.MustCompile(`^\s*[>❯›❱]\s*(\d{1,2})\.\s*(.+?)\s*$`)
	fallbackOptionRe      = regexp.MustCompile(`^\s*(\d{1,2})\.\s*(.+?)\s*$`)
	fallbackRadioOptionRe = regexp.MustCompile(`^\s*(\d{1,2})\s+\(([•●○*\-])\)\s+(.+?)\s*$`)
	labelDoubleSpaceTail  = regexp.MustCompile(`\s{2,}.*$`)
	labelGluedNextOption  = regexp.MustCompile(`\s*\d+\.\s*[A-Za-z].*$`)
	shortcutKeySuffixRe   = regexp.MustCompile(`(?i)\((y|p|n|!|#|\?|esc|escape)\)\s*$`)
	hubChoiceOptionLineRe = regexp.MustCompile(`^\s*(?:[>❯›❱]\s*)?\d{1,2}\.\s*\S`)
)

// fallbackOptions は extractApprovalOptions の結果。cluster は展開後の行（Lines）の位置。
type fallbackOptions struct {
	Options    []proto.ApprovalOption
	Lines      []string
	Start, End int
}

func buildFallbackApprovalOption(numText, labelText string, isCurrent bool) proto.ApprovalOption {
	label := strings.TrimSpace(labelText)
	label = labelDoubleSpaceTail.ReplaceAllString(label, "")
	label = strings.TrimSpace(labelGluedNextOption.ReplaceAllString(label, ""))
	n, _ := strconv.Atoi(numText)
	opt := proto.ApprovalOption{Num: n, Label: label, IsCurrent: isCurrent}
	if m := shortcutKeySuffixRe.FindStringSubmatch(label); m != nil {
		key := strings.ToLower(m[1])
		if key == "esc" || key == "escape" {
			key = "\x1b"
		}
		opt.SendText = key
	}
	return opt
}

// extractFallbackApprovalOptions は以前の approval-parser.ts の extractApprovalOptions を写したもの。
// 末尾に最も近い番号付きの選択肢の塊を下から集める。
func extractFallbackApprovalOptions(rawTail []string) fallbackOptions {
	tail := ungluedApprovalLines(rawTail)
	var options []proto.ApprovalOption
	clusterStart, clusterEnd := -1, -1
	seenOption := false
	blankGap := 0
	var pendingContinuation []string
	const maxBlankGap = 4
	consume := func(label string) string {
		if len(pendingContinuation) == 0 {
			return label
		}
		parts := make([]string, 0, len(pendingContinuation))
		for i := len(pendingContinuation) - 1; i >= 0; i-- {
			parts = append(parts, pendingContinuation[i])
		}
		pendingContinuation = nil
		return strings.Join(strings.Fields(label+" "+strings.Join(parts, " ")), " ")
	}
	add := func(i int, opt proto.ApprovalOption) {
		options = append([]proto.ApprovalOption{opt}, options...)
		if clusterEnd == -1 {
			clusterEnd = i
		}
		clusterStart = i
		seenOption = true
		blankGap = 0
	}
	for i := len(tail) - 1; i >= 0; i-- {
		line := tail[i]
		if m := fallbackCursorOptRe.FindStringSubmatch(line); m != nil {
			add(i, buildFallbackApprovalOption(m[1], consume(m[2]), true))
			continue
		}
		if m := fallbackOptionRe.FindStringSubmatch(line); m != nil {
			add(i, buildFallbackApprovalOption(m[1], consume(m[2]), false))
			continue
		}
		radio := strings.TrimSpace(strings.TrimRight(strings.TrimLeft(line, "│┃"), "│┃"))
		if m := fallbackRadioOptionRe.FindStringSubmatch(radio); m != nil {
			add(i, buildFallbackApprovalOption(m[1], consume(m[3]), m[2] != "○"))
			continue
		}
		if !seenOption {
			continue
		}
		if strings.TrimSpace(line) == "" {
			blankGap++
			if blankGap > maxBlankGap {
				break
			}
			pendingContinuation = nil
			continue
		}
		if blankGap == 0 {
			pendingContinuation = append(pendingContinuation, strings.TrimSpace(line))
			continue
		}
		break
	}
	empty := fallbackOptions{Lines: tail, Start: -1, End: -1}
	if len(options) == 0 {
		return empty
	}
	numMin, numMax := options[0].Num, options[0].Num
	for _, opt := range options {
		numMin = min(numMin, opt.Num)
		numMax = max(numMax, opt.Num)
	}
	if numMax > 20 || numMax-numMin > 15 || len(options) > 12 {
		return empty
	}
	seen := map[string]bool{}
	unique := make([]proto.ApprovalOption, 0, len(options))
	for _, opt := range options {
		key := strconv.Itoa(opt.Num) + ":" + opt.Label
		if seen[key] {
			continue
		}
		seen[key] = true
		unique = append(unique, opt)
	}
	if len(unique) < 2 {
		return empty
	}
	// AskUserQuestion のピッカーは Web ボタン化しない（approval_detector.go の同名の判定と同じ）。
	for _, opt := range unique {
		lower := strings.ToLower(strings.TrimSpace(opt.Label))
		if strings.HasPrefix(lower, "type something") || strings.HasPrefix(lower, "chat about") {
			return empty
		}
	}
	return fallbackOptions{Options: unique, Lines: tail, Start: clusterStart, End: clusterEnd}
}

func approvalContextLinesAround(lines []string, start, end, margin int) []string {
	if start < 0 {
		return lines
	}
	return lines[max(0, start-margin):min(len(lines), end+margin+1)]
}

// isHubChoicePrompt は以前の approval-parser.ts の同名関数を写したもの。質問が選択肢より前にあり、
// 自由入力の行（N. User specifies）があるときだけ旧形式の選択とみなす。
func isHubChoicePrompt(contextLines []string, options []proto.ApprovalOption) bool {
	if len(options) == 0 {
		return false
	}
	firstOption := -1
	for i, line := range contextLines {
		if hubChoiceOptionLineRe.MatchString(line) {
			firstOption = i
			break
		}
	}
	hasPrompt := false
	for i, line := range contextLines {
		if (firstOption == -1 || i < firstOption) && hubChoiceQuestionRe.MatchString(line) {
			hasPrompt = true
			break
		}
	}
	hasUserSpecifies := false
	for _, line := range contextLines {
		if userSpecifiesRe.MatchString(line) {
			hasUserSpecifies = true
			break
		}
	}
	for _, opt := range options {
		if userSpecifiesRe.MatchString(opt.Label) {
			hasUserSpecifies = true
		}
	}
	return hasPrompt && hasUserSpecifies
}

// markHubChoiceDefault は以前の approval-parser.ts の同名関数を写したもの。カーソルの無い旧形式の選択に
// 既定の選択（推奨、無ければ 1、それも無ければ先頭）を付ける。
func markHubChoiceDefault(options []proto.ApprovalOption) {
	for _, opt := range options {
		if opt.IsCurrent {
			return
		}
	}
	idx := -1
	for i, opt := range options {
		if recommendedChoiceRe.MatchString(opt.Label) {
			idx = i
			break
		}
	}
	if idx < 0 {
		for i, opt := range options {
			if opt.Num == 1 {
				idx = i
				break
			}
		}
	}
	if idx < 0 && len(options) > 0 {
		idx = 0
	}
	if idx >= 0 {
		options[idx].IsCurrent = true
	}
}

func extractHubChoiceQuestion(lines []string) *textQuestion {
	// 旧形式の選択は自由入力の行（N. User specifies）が必須（isHubChoicePrompt）。行の復元と
	// 選択肢の抽出は重いので、その文字が無い画面では走らせない（端末ミラーではチャンクごとに呼ぶ）。
	hasUserSpecifies := false
	for _, line := range lines {
		if userSpecifiesRe.MatchString(line) {
			hasUserSpecifies = true
			break
		}
	}
	if !hasUserSpecifies {
		return nil
	}
	found := extractFallbackApprovalOptions(lines)
	if len(found.Options) == 0 {
		return nil
	}
	context := approvalContextLinesAround(found.Lines, found.Start, found.End, 10)
	if !isHubChoicePrompt(context, found.Options) {
		return nil
	}
	question := ""
	for i := found.Start - 1; i >= max(0, found.Start-10); i-- {
		if hubChoiceQuestionRe.MatchString(found.Lines[i]) {
			question = strings.Join(strings.Fields(found.Lines[i]), " ")
			break
		}
	}
	options := append([]proto.ApprovalOption(nil), found.Options...)
	markHubChoiceDefault(options)
	// 旧形式の選択は文章で答えるので、送る文字列（ショートカット）は持たない。
	block := make([]string, 0, len(options)+2)
	if question != "" {
		block = append(block, question)
	}
	for i := range options {
		options[i].SendText = ""
		block = append(block, strconv.Itoa(options[i].Num)+". "+options[i].Label)
	}
	block = append(block, "N. User specifies")
	return newTextQuestion(approvalKindHubChoice, strings.Join(block, "\n"), question, options)
}

// ---- 連結された行の復元（approval-parser.ts の ungluedApprovalLines）----

var (
	userSpecifiesAnchorRe = regexp.MustCompile(`(?i)\s*\bN\.[ \t]*(User\s*specifies|その他指定)\b\s*`)
	numberedOptionStartRe = regexp.MustCompile(`^\s*\d{1,2}\.\s+\S`)
	strongGluedOptionRe   = regexp.MustCompile(`(\d{1,2})\.\s*\[([^\][\n]{1,16})\]`)
	bulletOnlyHeadRe      = regexp.MustCompile(`^[>❯›❱*\-•・]+$`)
)

func ungluedApprovalLines(lines []string) []string {
	afterQ := splitInlineQuestionHeading(lines)
	afterN := splitUserSpecifiesAnchor(afterQ)
	out := make([]string, 0, len(afterN))
	for _, line := range afterN {
		out = append(out, splitGluedNumberedLine(line)...)
	}
	return out
}

func splitUserSpecifiesAnchor(lines []string) []string {
	var out []string
	var expand func(line string)
	expand = func(line string) {
		loc := userSpecifiesAnchorRe.FindStringSubmatchIndex(line)
		if loc == nil {
			out = append(out, line)
			return
		}
		before := strings.TrimSpace(line[:loc[0]])
		after := strings.TrimSpace(line[loc[1]:])
		if before != "" {
			out = append(out, before)
		}
		kind := "User specifies"
		if strings.Contains(line[loc[2]:loc[3]], "その他指定") {
			kind = "その他指定"
		}
		out = append(out, "N. "+kind)
		if after != "" {
			expand(after)
		}
	}
	for _, raw := range lines {
		expand(raw)
	}
	return out
}

// inlineQuestionHeadingAt は s の中で、先頭の 1 文字より後にある最初の「Q/Ｑ + 数字 1〜2 桁」
// （直後が英数字でない）の開始位置を返す。JavaScript の /[QＱ]\d{1,2}(?![A-Za-z\d])/ を
// rest.slice(1) に掛けたものと同じ（Go の regexp には先読みが無いので手で書く）。
func inlineQuestionHeadingAt(s string) int {
	_, firstLen := utf8.DecodeRuneInString(s)
	for i := firstLen; i < len(s); {
		r, size := utf8.DecodeRuneInString(s[i:])
		if r == 'Q' || r == 'Ｑ' {
			j := i + size
			digits := 0
			for j < len(s) && s[j] >= '0' && s[j] <= '9' {
				j++
				digits++
			}
			if digits >= 1 && digits <= 2 {
				next, _ := utf8.DecodeRuneInString(s[j:])
				if j >= len(s) || !isASCIIAlnum(next) {
					return i
				}
			}
		}
		i += size
	}
	return -1
}

func isASCIIAlnum(r rune) bool {
	return (r >= 'A' && r <= 'Z') || (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9')
}

func splitInlineQuestionHeading(lines []string) []string {
	var out []string
	for _, raw := range lines {
		rest := raw
		if numberedOptionStartRe.MatchString(rest) {
			if strings.TrimSpace(rest) != "" {
				out = append(out, rest)
			}
			continue
		}
		for utf8.RuneCountInString(rest) > 1 {
			at := inlineQuestionHeadingAt(rest)
			if at < 0 {
				break
			}
			prev, _ := utf8.DecodeLastRuneInString(rest[:at])
			if strings.ContainsRune("[「『【（(", prev) {
				break
			}
			if head := strings.TrimSpace(rest[:at]); head != "" {
				out = append(out, head)
			}
			rest = rest[at:]
		}
		if strings.TrimSpace(rest) != "" {
			out = append(out, rest)
		}
	}
	return out
}

type gluedMark struct {
	at  int
	num int
}

// weakGluedMarks は JavaScript の
// /(^|[\s)）」』】。．？?！!…])(\d{1,2})\.(?:\s+|(?=\D))/g を line に掛けた結果（各一致の
// 番号の開始位置と番号）を返す。先読みを手で書くため、一致位置の進め方も exec と同じにする。
func weakGluedMarks(line string) []gluedMark {
	var marks []gluedMark
	isBoundary := func(r rune) bool {
		return unicode.IsSpace(r) || strings.ContainsRune(")）」』】。．？?！!…", r)
	}
	// matchDigitsAt は pos から「数字 1〜2 桁 + ピリオド + (空白の並び | 数字でない文字の手前)」を
	// 見て、一致すれば番号と一致の終わりを返す。
	matchDigitsAt := func(pos int) (int, int, bool) {
		j := pos
		for j < len(line) && line[j] >= '0' && line[j] <= '9' {
			j++
		}
		digits := j - pos
		if digits < 1 || digits > 2 || j >= len(line) || line[j] != '.' {
			return 0, 0, false
		}
		num, _ := strconv.Atoi(line[pos:j])
		k := j + 1
		if k >= len(line) {
			return 0, 0, false
		}
		r, _ := utf8.DecodeRuneInString(line[k:])
		if unicode.IsSpace(r) {
			for k < len(line) {
				r, size := utf8.DecodeRuneInString(line[k:])
				if !unicode.IsSpace(r) {
					break
				}
				k += size
			}
			return num, k, true
		}
		if r >= '0' && r <= '9' {
			return 0, 0, false
		}
		return num, k, true
	}
	for pos := 0; pos < len(line); {
		if pos == 0 {
			if num, end, ok := matchDigitsAt(0); ok {
				marks = append(marks, gluedMark{at: 0, num: num})
				pos = end
				continue
			}
		}
		r, size := utf8.DecodeRuneInString(line[pos:])
		if isBoundary(r) {
			if num, end, ok := matchDigitsAt(pos + size); ok {
				marks = append(marks, gluedMark{at: pos + size, num: num})
				pos = end
				continue
			}
		}
		pos += size
	}
	return marks
}

func splitGluedNumberedLine(line string) []string {
	var strong []gluedMark
	strongOK := true
	for _, loc := range strongGluedOptionRe.FindAllStringSubmatchIndex(line, -1) {
		num, _ := strconv.Atoi(line[loc[2]:loc[3]])
		if len(strong) > 0 && num <= strong[len(strong)-1].num {
			strongOK = false
			break
		}
		strong = append(strong, gluedMark{at: loc[0], num: num})
	}
	if strongOK && len(strong) >= 2 {
		return splitAtMarks(line, strong)
	}
	if len(strong) == 1 {
		head := strings.TrimSpace(line[:strong[0].at])
		seg := strings.TrimSpace(line[strong[0].at:])
		if head != "" && seg != "" && !(head[0] >= '0' && head[0] <= '9') {
			return []string{head, seg}
		}
	}
	marks := weakGluedMarks(line)
	if len(marks) < 2 {
		if len(marks) == 1 && marks[0].num == 1 {
			head := strings.TrimSpace(line[:marks[0].at])
			if head != "" && !bulletOnlyHeadRe.MatchString(head) {
				if seg := strings.TrimSpace(line[marks[0].at:]); seg != "" {
					return []string{head, seg}
				}
				return []string{head}
			}
		}
		return []string{line}
	}
	for i := 1; i < len(marks); i++ {
		if marks[i].num != marks[i-1].num+1 {
			return []string{line}
		}
	}
	return splitAtMarks(line, marks)
}

func splitAtMarks(line string, marks []gluedMark) []string {
	var out []string
	if head := strings.TrimSpace(line[:marks[0].at]); head != "" {
		out = append(out, head)
	}
	for i, mark := range marks {
		end := len(line)
		if i+1 < len(marks) {
			end = marks[i+1].at
		}
		if seg := strings.TrimSpace(line[mark.at:end]); seg != "" {
			out = append(out, seg)
		}
	}
	return out
}

// ---- 記録を開く ----

// openTextQuestion は次のユーザーの発話で答える質問の記録を開く。承認マーカーもここを通る
// （approval_marker.go の maybeBroadcastApprovalMarkerFrom）。開いたら true を返す。
func (s *Server) openTextQuestion(id int, q *textQuestion, detectedAt time.Time, source string) bool {
	if q == nil || q.Block == "" || q.Sig == "" {
		return false
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return false
	}
	// 画面に出ているネイティブの承認は、端末ミラーに残っている文章の質問より新しい
	// （approval_record.go 冒頭の例外）。世代を動かす前に判定する。
	if source == approvalSourceGoVT && approvalRecordBlocksVTMarkerLocked(ses) {
		s.sessionsMu.Unlock()
		return false
	}
	provider := ses.Provider
	identity := q.candidateIdentity(provider)
	sourceEpoch, answered := approvalCandidateEpochLocked(ses, identity.key)
	if answered || (ses.pendingApproval.isMarker() && ses.pendingApproval.isCandidate(identity.key, sourceEpoch)) {
		s.sessionsMu.Unlock()
		return false
	}
	superseded := closeApprovalRecordLocked(ses, id, approvalCloseSuperseded, detectedAt)
	superseded.dropActivity()
	opened, openedActivity := openApprovalRecordLocked(ses, id, &approvalRecord{
		CandidateKey:   identity.key,
		CandidateShape: identity.shape,
		SourceEpoch:    sourceEpoch,
		Sig:            q.Sig,
		Origin:         approvalRecordOriginMarker,
		Source:         source,
		Kind:           q.Kind,
		Block:          q.Block,
		Question:       q.Question,
		Options:        append([]proto.ApprovalOption(nil), q.Options...),
		DetectedAt:     detectedAt,
	})
	s.sessionsMu.Unlock()
	// 古い記録の resolved は新しい記録の INSERT より先に書く（approval_record.go 冒頭）。
	s.finishApprovalRecordClosures(superseded)

	// 台帳へ記録するのは配信するものだけ。承認マーカーの選択肢は解かずブロック原文を持つ
	// （マーカーの解釈器は画面の Hub ブロック用パーサの 1 本に保つ）。
	if s.sessionStore != nil {
		s.sessionStore.StoreApprovalDetected(sessionstore.ApprovalDetected{
			LiveSessionID: id,
			Sig:           q.Sig,
			Source:        source,
			Kind:          q.Kind,
			Provider:      provider,
			Question:      q.Question,
			Block:         q.Block,
			CandidateKey:  identity.key,
			SourceEpoch:   sourceEpoch,
			Options:       q.Options,
			DetectedAt:    detectedAt,
		})
	}

	s.broadcast(opened)
	if openedActivity != nil {
		s.broadcast(*openedActivity)
	}
	// 承認の通知は記録が開いたときの 1 回だけ。承認マーカーの本文は質問を渡さず、通知側の
	// 既定（最後の依頼など）に任せる（以前の evaluateIdle 経路と同じ）。
	question := q.Question
	if q.Kind == approvalKindMarker {
		question = ""
	}
	s.notifyApprovalPush(id, q.Sig, provider, question, "")
	s.notifyApprovalOutbound(id, q.Sig, provider, question, "")
	return true
}

// ---- 出力が落ち着いてから開く ----
//
// マーカー無しの文章の質問は、AI が話し終えてから（出力が idleAfter 止まってから）開く
// （C5 の敵対レビュー指摘 2・2026-09-23）。
//
// 承認マーカーは AI が「ここで答えを待つ」と書いた合図なので、見つけた時点で開く。文章の質問は
// 形から推しているだけで、作業の途中に書いた箇条書き（「C1:」の下に番号付きの箇条が 2 組）も
// 同じ形になる。途中で開くと Push が鳴り、次のユーザーターンまで「保留中」が残る。その間は
// オーケストレーションの board 通知と完了サマリーの代替も止まり、ユーザーターンが来ない無人の
// 子セッションでは止まったままになる。答えを待っている質問は、ターンの末尾にしか無い。
//
//   - 端末ミラー: チャンクごとには見ない。出力が落ち着いた時点（evaluateIdle の前）で画面を 1 回見る。
//   - トランスクリプト: 本文だけの assistant メッセージを候補（session.textQuestionAtIdle）にし、
//     落ち着いた時点で開く。すでに落ち着いていればその場で開く。候補の後に assistant の
//     メッセージ（本文・ツール・思考）が続いたら、AI は答えを待たずに話し続けているので候補を捨て、
//     開いている文章の質問も閉じる（approvalCloseVanished）。
//   - どちらも、user のメッセージか確定したユーザーターンが来たら候補を捨てる。
//
// 候補は「いま保留中」ではない（保留中は pendingApproval の 1 件だけ）。同じ質問かどうかの判定も
// 持たず、開くときに承認の同一性（approval_identity.go）を通る。

// isTranscriptTextQuestion は、トランスクリプトから開いたマーカー無しの文章の質問の記録かを返す。
func (r *approvalRecord) isTranscriptTextQuestion() bool {
	return r.isMarker() && r.Kind != approvalKindMarker && r.Source == approvalSourceTranscript
}

// observeTranscriptTextQuestion は、トランスクリプトの assistant メッセージ 1 通（承認マーカーを
// 含まないもの）を見たときに呼ぶ。q はそのメッセージが文章の質問ならその質問、違えば nil。
// 開いている文章の質問は、同じ質問が続いたのでなければ閉じる。
func (s *Server) observeTranscriptTextQuestion(id int, q *textQuestion, detectedAt time.Time) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	var closure *approvalRecordClosure
	if record := ses.pendingApproval; record.isTranscriptTextQuestion() &&
		(q == nil || q.candidateIdentity(ses.Provider).key != record.CandidateKey) {
		closure = closeApprovalRecordLocked(ses, id, approvalCloseVanished, detectedAt)
	}
	ses.textQuestionAtIdle = q
	openNow := q != nil && ses.Activity.OutputIdle
	if openNow {
		ses.textQuestionAtIdle = nil
	}
	s.sessionsMu.Unlock()
	s.finishApprovalRecordClosures(closure)
	if openNow {
		s.openTextQuestion(id, q, detectedAt, approvalSourceTranscript)
	}
}

// openTextQuestionsOnOutputIdle は、出力がいま落ち着いたセッションで文章の質問を 1 回だけ見て開く。
// evaluateIdle が状態を決める前に呼ぶ。先に開いておくと同じ評価で「保留中」になり、standby への
// 遷移として完了サマリーの代替を作らない。
func (s *Server) openTextQuestionsOnOutputIdle(now time.Time) {
	type opening struct {
		id     int
		q      *textQuestion
		source string
	}
	var opens []opening
	s.sessionsMu.Lock()
	for id, ses := range s.sessions {
		if isTerminalSessionState(ses.State) || ses.Activity.OutputIdle {
			continue
		}
		if !ses.lastOutputAt.IsZero() && now.Sub(ses.lastOutputAt) < idleAfter {
			continue
		}
		held := ses.textQuestionAtIdle
		ses.textQuestionAtIdle = nil
		if approvalMarkerSourceIsTranscriptLocked(ses) {
			if held != nil {
				opens = append(opens, opening{id: id, q: held, source: approvalSourceTranscript})
			}
			continue
		}
		if q := vtTextQuestionLocked(ses, now); q != nil {
			opens = append(opens, opening{id: id, q: q, source: approvalSourceGoVT})
		}
	}
	s.sessionsMu.Unlock()
	for _, o := range opens {
		s.openTextQuestion(o.id, o.q, now, o.source)
	}
}

// vtTextQuestionLocked は端末ミラーの現在画面からマーカー無しの文章の質問を探す。承認マーカーが
// 読めるときはそちらが優先なので探さない（マーカーはチャンクごとの経路が開く）。
// トランスクリプトが供給元のセッションでは呼ばない（approval_marker_transcript.go の 1 供給元の規則）。
func vtTextQuestionLocked(ses *session, now time.Time) *textQuestion {
	if ses.vt == nil || !sessionApprovalDetectionEligible(ses) || now.Before(ses.vtResizeDebounceUntil) {
		return nil
	}
	if extractApprovalMarkerBlockFromVT(ses.vt) != nil {
		return nil
	}
	return detectTextQuestion(ses.vt.TailLines(vtTailLinesForApproval))
}

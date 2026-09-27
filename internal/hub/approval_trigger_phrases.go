package hub

import (
	"bytes"
	"strings"

	"many-ai-cli/internal/config"
)

// approval_trigger_phrases.go: 承認らしさの手がかり語を Hub の画面検出へ渡す。
//
// これまで provider 別の手がかり語（~/.many-ai-cli/approval-patterns/ の provider 別のファイルと
// 全 provider 共通のファイル。利用者が設定画面で足せる）はブラウザの推測検出（approval.ts の
// matchProviderApprovalTrigger）だけが読んでいて、Hub の画面検出（detectNativeApproval）は
// 固定の語だけを見ていた。そのためブラウザだけが拾える承認画面があった（親 plan の D1 で
// 「推測での拾い上げ」と呼んだもの）。Hub がこの語も読み、承認の記録を Hub だけで作れるようにする。
//
// 読むのは Hub の起動時と、ファイルを書き換える経路（設定画面・公式の取得・custom provider の
// 取得）の直後だけ。検出はロックの中でも走るので、検出のたびにファイルを読まない。利用者が
// ファイルを手で書き換えた場合は、ブラウザと同じく次の起動（ブラウザは再読み込み）で効く。

// approvalTriggerPhraseSet は provider → 小文字化した手がかり語。common は全 provider に効く。
type approvalTriggerPhraseSet map[string][]string

// webNativeApprovalTriggerPhrases は approval-parser.ts の matchNativeApprovalTrigger と同じ語。
// ブラウザはこれを provider を問わず承認の手がかりにしている。Hub 側の既定の手がかり語
// （nativeApprovalLooksValid）と重なる語もあるが、欠けていた語を拾うためにそのまま持つ。
var webNativeApprovalTriggerPhrases = []string{
	"requires approval",
	"would you like to run the following command",
	"would you like to run",
	"do you want to proceed?",
	"this command requires approval",
	"permission required",
	"permissions required",
	"requires permission",
	"requires confirmation",
	"prompts for user confirmation",
	"allow all similar",
	"deny all similar",
	"enter to select",
	"enter select",
	"↑/↓ to navigate",
	"do you trust the files in this folder",
	"tool permission",
	"command code needs to run",
	"do you want to make this edit",
	"esc to cancel",
	"tab:next option",
	"always-approve mode",
	"type to add feedback",
	"ctrl+o:always-approve",
}

// reloadApprovalTriggerPhrases はパターンファイルを読み直す。ファイル IO をするので、
// sessionsMu などのロックを持ったまま呼ばない。
func (s *Server) reloadApprovalTriggerPhrases() {
	set := approvalTriggerPhraseSet{}
	for _, provider := range KnownApprovalProviders() {
		set[provider] = normalizeApprovalTriggerPhrases(readCustomApprovalPatternsMirror(provider))
	}
	var customIDs []string
	if s.cfg != nil {
		s.cfgMu.Lock()
		for _, p := range config.EffectiveCustomProviders(s.cfg.CustomProviders) {
			customIDs = append(customIDs, p.ID)
		}
		s.cfgMu.Unlock()
	}
	for _, id := range customIDs {
		if _, builtIn := set[id]; builtIn || id == "" {
			continue
		}
		set[id] = normalizeApprovalTriggerPhrases(readCustomApprovalPatternsMirror(id))
	}
	s.approvalTriggerPhrases.Store(&set)
}

func normalizeApprovalTriggerPhrases(list []string) []string {
	out := make([]string, 0, len(list))
	for _, phrase := range list {
		if phrase = strings.ToLower(strings.TrimSpace(phrase)); phrase != "" {
			out = append(out, phrase)
		}
	}
	return out
}

// approvalTriggerPhrasesFor は provider 固有の語と common の語を返す。読み込み前（テストの
// Server を含む）は nil を返し、検出は既定の手がかり語だけで動く。
func (s *Server) approvalTriggerPhrasesFor(provider string) []string {
	set := s.approvalTriggerPhrases.Load()
	if set == nil {
		return nil
	}
	own := (*set)[provider]
	common := (*set)["common"]
	if len(own) == 0 {
		return common
	}
	if len(common) == 0 {
		return own
	}
	out := make([]string, 0, len(own)+len(common))
	out = append(out, own...)
	return append(out, common...)
}

// lineMatchesApprovalTrigger は approval.ts の matchProviderApprovalTrigger と
// matchNativeApprovalTrigger を合わせたもの。
func lineMatchesApprovalTrigger(provider, line string, phrases []string) bool {
	lower := strings.ToLower(line)
	if lower == "" {
		return false
	}
	if webNativeApprovalTrigger(lower) {
		return true
	}
	// モデル選択画面の行は provider 固有の語で拾わない（codex / opencode の /model は承認ではない）。
	if isModelSelectorHintLine(provider, lower) {
		return false
	}
	for _, phrase := range phrases {
		if strings.Contains(lower, phrase) {
			return true
		}
	}
	return false
}

// webNativeApprovalTrigger は approval-parser.ts の matchNativeApprovalTrigger（小文字化済みの行）。
func webNativeApprovalTrigger(lower string) bool {
	for _, phrase := range webNativeApprovalTriggerPhrases {
		if strings.Contains(lower, phrase) {
			return true
		}
	}
	// codex の /model 画面のフッター（press enter to confirm or esc to go back）は除く。
	return strings.Contains(lower, "press enter to confirm") && !strings.Contains(lower, "esc to go back")
}

// isModelSelectorHintLine は、モデル選択画面の行かを返す（以前 approval.ts にあった isModelSelectorHint を写したもの）。
func isModelSelectorHintLine(provider, lower string) bool {
	switch provider {
	case "codex":
		return strings.Contains(lower, "select model") ||
			strings.Contains(lower, "select effort") ||
			strings.Contains(lower, "model and effort") ||
			strings.Contains(lower, "reasoning effort") ||
			strings.Contains(lower, "esc to go back") ||
			strings.Contains(lower, "↑/↓ to change") ||
			strings.Contains(lower, "arrow keys")
	case "opencode":
		return strings.Contains(lower, "select model") ||
			strings.Contains(lower, "connect provider") ||
			strings.Contains(lower, "favorite ctrl+f") ||
			strings.Contains(lower, "opencode zen") ||
			strings.Contains(lower, "ollama (local)")
	}
	return false
}

// screenApprovalScanPhrases は画面検出を走らせる入口で見る語のうち、固定の
// nativeApprovalTriggerTokens に無いもの（小文字）。AskUserQuestion の告知と、
// 「N. User specifies」を手がかりにする承認画面の分。
var screenApprovalScanPhrases = []string{
	"review your answers",
	"ready to submit your answers",
	"submit",
	"type something",
	"chat about this",
	"user specifies",
	"その他指定",
}

// chunkMayShowScreenApproval は、この PTY チャンクで画面検出を走らせるかを返す。
// 固定の語（nativeApprovalTriggerTokens）に加えて、ブラウザの推測検出の手がかり語・
// 利用者の承認パターン・AskUserQuestion の語を見る。
func (s *Server) chunkMayShowScreenApproval(provider string, data []byte) bool {
	if ptyChunkContainsAny(data, nativeApprovalTriggerTokens) {
		return true
	}
	lower := bytes.ToLower(data)
	return lowerContainsAnyPhrase(lower, webNativeApprovalTriggerPhrases) ||
		lowerContainsAnyPhrase(lower, screenApprovalScanPhrases) ||
		lowerContainsAnyPhrase(lower, s.approvalTriggerPhrasesFor(provider))
}

// lowerContainsAnyPhrase は小文字化済みのチャンクに、小文字の語のどれかが含まれるかを返す。
func lowerContainsAnyPhrase(lower []byte, phrases []string) bool {
	for _, phrase := range phrases {
		if bytes.Contains(lower, []byte(phrase)) {
			return true
		}
	}
	return false
}

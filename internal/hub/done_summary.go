package hub

import (
	"fmt"
	"regexp"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"many-ai-cli/internal/notify"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const doneSummaryMaxRunes = 320

// inputBoxSearchDepth は入力ボックスの下辺を探しに行く画面下端からの行数。
// ボックスの下にぶら下がる案内行（"? for shortcuts" 等）とスクロール余白を
// 跨げる程度に取り、本文中の枠線まで拾わない程度に浅くする。
const inputBoxSearchDepth = 8

// publishDoneSummary makes completion information visible in the Hub even
// when external notifications are disabled. External delivery remains an
// explicit opt-in user preference.
func (s *Server) publishDoneSummary(summary proto.DoneSummary) {
	s.publishDoneSummaryInternal(summary, true)
}

// publishRelayDoneSummary sends a relay terminal notification through the
// same browser / optional external channels as an ordinary completion, but it
// must not start a Git-turn snapshot for the parent session. Relay work is
// already represented by its own worktree or shared-tree state.
func (s *Server) publishRelayDoneSummary(summary proto.DoneSummary) {
	s.publishDoneSummaryInternal(summary, false)
}

func (s *Server) publishDoneSummaryInternal(summary proto.DoneSummary, captureGit bool) {
	summary.Text = truncateDoneSummary(sessionlog.MaskSecrets(summary.Text))
	if summary.Text == "" {
		return
	}
	if summary.Kind == "" {
		summary.Kind = classifyDoneSummary(summary.Text)
	}
	if summary.At == "" {
		summary.At = time.Now().Format(time.RFC3339)
	}
	if captureGit {
		// 次ターン入力が DONE 通知直後に届いても baseline を取りこぼさないよう、
		// 完了 snapshot を「処理中」と同期的に確定してから UI へ通知する。
		// Git I/O 自体は captureGitTurnEnd が内部 goroutine で行う。
		s.captureGitTurnEnd(summary.SessionID, summary.At)
	}
	// handoff は log.session_enabled とも通知設定とも無関係（handoff.go 参照）。
	// 外部通知が off の利用者でも看板には残す。
	s.recordHandoffDone(summary)
	// 子 plan C3 内部 C2（docs/local/plan_session-handoff-board_c3_intent-layer.md）:
	// DONE の任意行「次:」「未検証:」を拾えたときだけ kind=intent を追記する。
	// 切り出せなかったターンは何も書かない（空の意図行を作らない）。
	if next, unverified, ok := extractIntentFromDoneText(summary.Text); ok {
		s.recordHandoffIntent(summary.SessionID, next, unverified)
	}
	s.broadcast(proto.Message{Type: "done_summary", SessionID: summary.SessionID, Provider: summary.Provider, DoneSummary: &summary})

	if !s.shouldNotifyDoneExternally(summary) {
		return
	}
	s.notifyDoneOutbound(summary)
	s.notifyDonePush(summary)
}

// shouldNotifyDoneExternally は完了サマリーを外部（notify backends / Web Push）へ
// 流すかを決める。**フォールバックは Hub の中だけで見せ、外へは出さない。**
//
// フォールバックが言えるのは「ターンが終わった」ことだけで、何が終わったかは
// 分からない。マーカーの規約が「通常の会話・質問への回答には出力しない」と
// 定めている以上、**マーカーが無いことは異常ではない**（規約の正本は
// internal/wrapper/approval_rules.go の rulesFileContent）。会話ターンごとに
// push が飛ぶと、本物の完了通知のほうが埋もれる。
//
// マーカーを出さない provider（Codex は 2026-08-26 実測で 3 セッション連続 0 件）
// には、この抑止で完了通知が届かなくなる。**それはフォールバックを鳴らし続けて
// 埋める話ではなく、その provider の完了を実際に検出する話**として別に扱う
// （docs/local/bugfix_done-summary-fallback-false-positive_2026-08-26.md の論点 A）。
func (s *Server) shouldNotifyDoneExternally(summary proto.DoneSummary) bool {
	if summary.Fallback {
		return false
	}
	return s.doneSummaryNotifyEnabled()
}

// doneSummaryNotifyEnabled は完了サマリーを外部へ通知するかを決める。
//
// 利用者が設定を触っていれば（true / false どちらでも）その値に従う。
// **未設定のときだけ**、通知チャネルを既に持っているかで決める。
//
// これは v0.7.0 で「既定 OFF」から変えたもの。README は「止まった瞬間を届ける」と
// 言っているのに、完了通知が 2 段のオプトインの奥にあって既定では届いていなかった
// （docs/local/plan_xirp-automode-positioning.md C4）。
//
// **通知手段を持たない利用者の挙動は変えない。** Web Push も notify.backends も
// 無い環境では、既定 ON にしても送る先が無く、何も増えない。既定を変えるのは
// 「通知を受け取る用意が既にある」と表明済みの利用者に対してだけにする。
func (s *Server) doneSummaryNotifyEnabled() bool {
	s.cfgMu.Lock()
	explicit := s.cfg.UserPrefs.DoneSummaryNotify.Enabled
	backends := len(s.cfg.Notify.Backends)
	s.cfgMu.Unlock()
	if explicit != nil {
		return *explicit
	}
	if backends > 0 {
		return true
	}
	return s.hasPushSubscription()
}

// hasPushSubscription は Web Push の購読が 1 件以上あるかを返す。
// push manager が初期化されていない構成（テスト等）では false。
func (s *Server) hasPushSubscription() bool {
	if s.push == nil {
		return false
	}
	return s.push.status().Subscriptions > 0
}

// intentNextPattern / intentUnverifiedPattern pull the DONE format's optional
// "次:" / "未検証:" lines (internal/wrapper/approval_rules.go rulesFileContent,
// version 21) out of an already-sanitized summary.Text. By the time this runs,
// truncateDoneSummary has already joined every original line into a single
// space-separated string (strings.Fields + Join), so both labels are matched
// as a token preceded by the string start or whitespace, captured up to the
// next label or the end of the line — there are no newlines left to split on.
var (
	intentNextPattern       = regexp.MustCompile(`(?:^|\s)次:\s*(.+?)(?:\s+未検証:|$)`)
	intentUnverifiedPattern = regexp.MustCompile(`(?:^|\s)未検証:\s*(.+)$`)
)

// extractIntentFromDoneText 子 plan C3 内部 C2 の切り出し本体。ラベルが 1 つも
// 見つからなければ ok=false を返す。呼び出し側はこれで「今回は意図なし」を
// 判定し、空文字の kind=intent を看板へ書かない（子 plan 「空の行を作らない」）。
func extractIntentFromDoneText(text string) (next, unverified string, ok bool) {
	if m := intentNextPattern.FindStringSubmatch(text); m != nil {
		next = strings.TrimSpace(m[1])
	}
	if m := intentUnverifiedPattern.FindStringSubmatch(text); m != nil {
		unverified = strings.TrimSpace(m[1])
	}
	if next == "" && unverified == "" {
		return "", "", false
	}
	return next, unverified, true
}

func classifyDoneSummary(text string) string {
	lower := strings.ToLower(text)
	switch {
	case strings.Contains(lower, "要判断"), strings.Contains(lower, "要対応"), strings.Contains(lower, "確認が必要"), strings.Contains(lower, "blocked"), strings.Contains(lower, "判断が必要"):
		return "needs_action"
	case strings.Contains(lower, "中断"), strings.Contains(lower, "キャンセル"), strings.Contains(lower, "aborted"), strings.Contains(lower, "cancelled"), strings.Contains(lower, "canceled"):
		return "aborted"
	case strings.Contains(lower, "失敗"), strings.Contains(lower, "エラー"), strings.Contains(lower, "failed"), strings.Contains(lower, "error"):
		return "failure"
	default:
		return "success"
	}
}

func truncateDoneSummary(text string) string {
	text = strings.Join(strings.Fields(strings.TrimSpace(text)), " ")
	if utf8.RuneCountInString(text) <= doneSummaryMaxRunes {
		return text
	}
	runes := []rune(text)
	return strings.TrimSpace(string(runes[:doneSummaryMaxRunes])) + "…"
}

// maybeCreateFallbackDoneSummary is called only on a running -> standby
// transition. A fallback is emitted only after the Git turn snapshot proves
// that the just-finished confirmed user turn changed at least one file. This
// keeps ordinary conversation turns out of the fallback path and gives the
// asynchronous Git capture a single work-turn predicate.
func (s *Server) maybeCreateFallbackDoneSummary(id int) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !isFallbackDoneSummaryProvider(ses.Provider) || ses.approvalVisible {
		s.sessionsMu.Unlock()
		return
	}
	title := doneSummaryTitle(ses)
	// 表示に使うのは vt が再構成した画面行であって doneMsgBuf ではない。
	// doneMsgBuf はマーカーがチャンク境界をまたぐのを吸収するための連結
	// バッファで、人間が読む文字列の供給源ではない（下の lastUsefulDoneLine
	// の doc 参照）。
	var screen []string
	if ses.vt != nil {
		screen = ses.vt.Lines()
	}
	provider := ses.Provider
	s.sessionsMu.Unlock()

	s.captureGitTurnEndWithCallback(id, now.Format(time.RFC3339), func(turn gitTurnSnapshot) {
		if turn.Files == 0 {
			return
		}
		s.sessionsMu.Lock()
		current := s.sessions[id]
		// A real marker may have arrived while the Git capture was running. The
		// marker handler owns the same timestamp, so only suppress a fallback
		// that was superseded after this fallback candidate was scheduled.
		if current == nil || !isFallbackDoneSummaryProvider(current.Provider) || current.approvalVisible || current.doneSummaryMarkerSeen || current.lastDoneNotifyAt.After(now) {
			s.sessionsMu.Unlock()
			return
		}
		current.lastDoneNotifyAt = now
		s.sessionsMu.Unlock()

		text := fallbackDoneSummaryText(lastUsefulDoneLine(screen))
		s.publishDoneSummary(proto.DoneSummary{SessionID: id, Provider: provider, Title: title, Text: text, Kind: fallbackDoneSummaryKind, At: now.Format(time.RFC3339), Fallback: true})
	})
}

// Codex has a provider-owned task_complete signal. Its fallback is disabled
// so a missing marker cannot race that real completion path into a duplicate
// card; providers without such a signal still use the Git-change predicate.
func isFallbackDoneSummaryProvider(provider string) bool {
	return isAIProvider(provider) && provider != "codex"
}

// handleCodexTaskCompletion turns Codex's provider-owned task_complete event
// into a normal completion summary. It still uses the confirmed user-turn Git
// baseline, so a conversational Codex response does not become a completion
// notification merely because the rollout says that the response ended.
func (s *Server) handleCodexTaskCompletion(id int, completion codexTaskCompletion) {
	endedAt, err := time.Parse(time.RFC3339, completion.At)
	if err != nil {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.Provider != "codex" {
		s.sessionsMu.Unlock()
		return
	}
	title := doneSummaryTitle(ses)
	s.sessionsMu.Unlock()

	s.captureGitTurnEndWithCallback(id, completion.At, func(turn gitTurnSnapshot) {
		if turn.Files == 0 {
			return
		}
		s.sessionsMu.Lock()
		current := s.sessions[id]
		if current == nil || current.Provider != "codex" || current.lastDoneNotifyAt.After(endedAt) {
			s.sessionsMu.Unlock()
			return
		}
		current.lastDoneNotifyAt = endedAt
		s.sessionsMu.Unlock()

		text := completion.LastAgentMessage
		if strings.TrimSpace(text) == "" {
			text = "Codex ターン完了"
		}
		s.publishDoneSummary(proto.DoneSummary{
			SessionID: id,
			Provider:  "codex",
			Title:     title,
			Text:      text,
			Kind:      classifyDoneSummary(text),
			At:        completion.At,
		})
	})
}

// fallbackDoneSummaryKind はフォールバックが名乗る kind。
//
// **needs_action にしない。** マーカーの規約は「タスク完了時のみ出力」なので、
// 会話ターンにマーカーが無いのは正常動作で、そこへ要対応の色を出すと毎ターン
// 誤報になる。success 側（既定）にも寄せない。何が終わったかは本当に分からない。
const fallbackDoneSummaryKind = "unknown"

// fallbackDoneSummaryText はフォールバックの文面を組む。
//
// 「異常が起きた」ではなく「何が終わったか分からない」と読める語彙にする。
// classifyDoneSummary へ通さないのは、この文字列が AI の報告ではなく Hub の
// 定型文 + 画面の最終行だから（最終行の語彙で success / failure に振れてしまう）。
func fallbackDoneSummaryText(last string) string {
	if last == "" {
		return "ターン終了（完了サマリーなし）"
	}
	return fmt.Sprintf("ターン終了（完了サマリーなし）。最後の出力: %s", last)
}

// lastUsefulDoneLine は「画面に実際に描かれている最後の 1 行」を返す。
//
// **引数は vtBuffer が再構成した画面行であり、PTY の生ストリームではない。**
// ここを取り違えると壊れる: StripANSI 済みのストリームはカーソル移動が落ちて
// いるので、TUI が同じ行を上書き再描画した断片が改行なしで連結する。その
// 連結物の「最後の行」は日本語として成立しない混合物になり、しかも
// publishDoneSummary は notifyDonePush まで通すので push 通知にも乗る
// （docs/local/bugfix_fallback-done-summary-garbled-tail_2026-08-26.md）。
//
// 画面下端は入力ボックスとショートカット案内で、利用者が打ちかけた入力も
// そこに居る。AI の出力ではないので、ボックスの上辺より上だけを走査する。
func lastUsefulDoneLine(screen []string) string {
	end := len(screen)
	if top := inputBoxTopIndex(screen); top >= 0 {
		end = top
	}
	for i := end - 1; i >= 0; i-- {
		raw := strings.TrimSpace(screen[i])
		if strings.HasPrefix(raw, "│") || strings.HasPrefix(raw, "┃") {
			// 枠線で囲まれたパネルの中身。入力ボックスの上辺を見つけられ
			// なかったとき（上端が画面外へ流れたとき）の受け皿。
			continue
		}
		line := strings.TrimSpace(cleanTUILine(screen[i]))
		if line == "" || strings.HasPrefix(line, "[") || hasSpinnerPrefix(line) || isDoneChromeLine(line) {
			continue
		}
		return truncateDoneSummary(line)
	}
	return ""
}

// inputBoxTopIndex は画面最下部の入力ボックスの「上辺」の行番号を返す（無ければ -1）。
//
// 下辺を先に探してから上辺へ遡る。こうするとボックスより下に居る
// ショートカット案内（"? for shortcuts" 等）も一緒に走査対象から外れる。
// 案内文はプロバイダごとに文言が違うので、語で弾かず構造で外す。
// 上辺が画面外へ流れていた場合は下辺の位置を返し、中身は
// lastUsefulDoneLine 側の縦罫線スキップで落とす。
func inputBoxTopIndex(screen []string) int {
	// 下辺の探索は最下部だけに限る。上の方に出てくる表や引用の枠を
	// 入力ボックスと取り違えないための制限。
	bottom := -1
	for i := len(screen) - 1; i >= 0 && i >= len(screen)-inputBoxSearchDepth; i-- {
		if isBoxBorderLine(strings.TrimSpace(screen[i]), "╰└┗╚") {
			bottom = i
			break
		}
	}
	if bottom < 0 {
		return -1
	}
	for i := bottom - 1; i >= 0; i-- {
		if isBoxBorderLine(strings.TrimSpace(screen[i]), "╭┌┏╔") {
			return i
		}
	}
	return bottom
}

// isBoxBorderLine は「先頭が corners のいずれかで、かつ装飾文字だけで
// できている行」を枠線とみなす。
func isBoxBorderLine(line string, corners string) bool {
	if line == "" {
		return false
	}
	first, _ := utf8.DecodeRuneInString(line)
	if !strings.ContainsRune(corners, first) {
		return false
	}
	return isDoneChromeLine(line)
}

// isDoneChromeLine は罫線・区切り線・入力プロンプト記号だけの行を判定する。
// 意味のある文字が 1 つでもあれば false。
func isDoneChromeLine(line string) bool {
	for _, r := range line {
		switch {
		case unicode.IsSpace(r):
		case r >= 0x2500 && r <= 0x259F: // Box Drawing + Block Elements
		case r == '-', r == '=', r == '_', r == '~':
		case r == '>', r == '$', r == '❯', r == '›':
		default:
			return false
		}
	}
	return true
}

// hasSpinnerPrefix は行頭がブライユ点字のスピナー（⠋ ⠙ …）かを見る。
// スピナー行は再描画の途中経過であって完了時の出力ではない。
func hasSpinnerPrefix(line string) bool {
	r, _ := utf8.DecodeRuneInString(line)
	return r >= 0x2800 && r <= 0x28FF
}

func doneSummaryTitle(ses *session) string {
	title := strings.TrimSpace(ses.Display)
	if title == "" {
		title = strings.TrimSpace(ses.Provider)
	}
	if title == "" {
		title = "many-ai-cli"
	}
	if ses.Label != "" {
		return fmt.Sprintf("%s #%d [%s]", title, ses.ID, ses.Label)
	}
	return fmt.Sprintf("%s #%d", title, ses.ID)
}

// notifyDoneOutbound uses the same backend route as approval notifications,
// but carries the normalized kind so ntfy/webhook clients can render the four
// terminal states consistently.
func (s *Server) notifyDoneOutbound(summary proto.DoneSummary) {
	if s.notifyMgr == nil {
		return
	}
	s.notifyMgr.SendDone(notify.DonePayload{SessionID: summary.SessionID, Provider: summary.Provider, Title: summary.Title, Summary: summary.Text, Kind: summary.Kind})
}

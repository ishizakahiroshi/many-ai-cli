package hub

import (
	"fmt"
	"strings"
	"time"
	"unicode/utf8"

	"many-ai-cli/internal/notify"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const doneSummaryMaxRunes = 320

// publishDoneSummary makes completion information visible in the Hub even
// when external notifications are disabled. External delivery remains an
// explicit opt-in user preference.
func (s *Server) publishDoneSummary(summary proto.DoneSummary) {
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
	// 次ターン入力が DONE 通知直後に届いても baseline を取りこぼさないよう、
	// 完了 snapshot を「処理中」と同期的に確定してから UI へ通知する。
	// Git I/O 自体は captureGitTurnEnd が内部 goroutine で行う。
	s.captureGitTurnEnd(summary.SessionID, summary.At)
	s.broadcast(proto.Message{Type: "done_summary", SessionID: summary.SessionID, Provider: summary.Provider, DoneSummary: &summary})

	if !s.doneSummaryNotifyEnabled() {
		return
	}
	s.notifyDoneOutbound(summary)
	s.notifyDonePush(summary)
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
// transition. This prevents periodic idle ticks from creating duplicate DONE
// events, while still covering providers that omit the marker entirely.
func (s *Server) maybeCreateFallbackDoneSummary(id int) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !isAIProvider(ses.Provider) || ses.approvalVisible || ses.nativeApprovalSig != "" || (!ses.lastDoneNotifyAt.IsZero() && now.Sub(ses.lastDoneNotifyAt) < doneNotifyMinInterval) {
		s.sessionsMu.Unlock()
		return
	}
	ses.lastDoneNotifyAt = now
	title := doneSummaryTitle(ses)
	tail := ses.doneMsgBuf.String()
	provider := ses.Provider
	s.sessionsMu.Unlock()

	last := lastUsefulDoneLine(tail)
	text := "完了マーカーを検出できませんでした。最後の出力を確認してください。"
	if last != "" {
		text = fmt.Sprintf("完了マーカーを検出できませんでした。最後の出力: %s", last)
	}
	s.publishDoneSummary(proto.DoneSummary{SessionID: id, Provider: provider, Title: title, Text: text, Kind: "needs_action", At: now.Format(time.RFC3339), Fallback: true})
}

func lastUsefulDoneLine(tail string) string {
	lines := strings.Split(tail, "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		line := strings.TrimSpace(lines[i])
		if line == "" || strings.HasPrefix(line, "[") || strings.HasPrefix(line, "⠋") {
			continue
		}
		return truncateDoneSummary(line)
	}
	return ""
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

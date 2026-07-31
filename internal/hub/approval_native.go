package hub

// approval_native.go: server.go から分離した「ネイティブ承認検出まわり」の関数群。
//
// C4 (plan_audit_score_s_promotion_2026-07-05.md): server.go 3144 行を関心事別に
// 割るリファクタの第一弾。以下の 9 関数は「Go 側 VT ミラーで検出した provider
// 承認プロンプト」の状態機械 (nativeApprovalSig / ClearMisses / DoneSummary
// マーカー処理・PTY replay バッファ) を扱う一塊で、他の関心事から明確に分離
// できる。挙動は移動前と完全に同一・全て package-private・呼び出し元は変更なし。
//
// 分割対象外 (server.go 側に残る) は詳細説明・model_detect / input_gate /
// idle_state / branch_refresh / ui_broadcast / wrapper_loop 系。同 package なので
// 型・定数・他関数への参照はそのまま通る。

import (
	"bytes"
	"fmt"
	"hash/fnv"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

func (s *Server) resetNativeApprovalClearMisses(id int) {
	s.sessionsMu.Lock()
	if ses := s.sessions[id]; ses != nil {
		ses.nativeApprovalClearMisses = 0
	}
	s.sessionsMu.Unlock()
}

func (s *Server) handleNativeApprovalDetection(id int, approval *nativeApproval) {
	now := time.Now()
	var msg *proto.Message
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	if now.Before(ses.vtResizeDebounceUntil) {
		s.sessionsMu.Unlock()
		return
	}
	if approval == nil {
		if ses.nativeApprovalSig != "" {
			ses.nativeApprovalClearMisses++
			if ses.nativeApprovalClearMisses >= nativeApprovalClearMissLimit {
				sig := ses.nativeApprovalSig
				ses.nativeApprovalSig = ""
				ses.nativeApprovalClearMisses = 0
				msg = &proto.Message{
					Type:           "approval_cleared",
					SessionID:      id,
					Provider:       ses.Provider,
					ApprovalSig:    sig,
					ApprovalSource: approvalSourceGoVT,
				}
			}
		}
		s.sessionsMu.Unlock()
		if msg != nil {
			s.broadcast(*msg)
		}
		return
	}
	if ses.nativeApprovalConsumed == approval.Sig && now.Sub(ses.nativeApprovalConsumedAt) < approvalConsumedTTL {
		s.sessionsMu.Unlock()
		return
	}
	ses.nativeApprovalClearMisses = 0
	if ses.nativeApprovalSig != approval.Sig {
		ses.nativeApprovalSig = approval.Sig
		msg = &proto.Message{
			Type:             "approval_detected",
			SessionID:        id,
			Provider:         ses.Provider,
			ApprovalSig:      approval.Sig,
			ApprovalKind:     approval.Kind,
			ApprovalSource:   approvalSourceGoVT,
			ApprovalQuestion: approval.Question,
			ApprovalContext:  approval.Context,
			ApprovalOptions:  approval.Options,
			ApprovalSummary:  &approval.Summary,
			DetectedAt:       now.Format(time.RFC3339),
		}
	}
	s.sessionsMu.Unlock()
	if msg != nil {
		if msg.Type == "approval_detected" && s.maybeAutoApprove(id, approval) {
			return
		}
		if s.sessionStore != nil && msg.Type == "approval_detected" {
			s.sessionStore.StoreApprovalDetected(id, approval.Sig, approvalSourceGoVT, approval.Kind, approval.Question, approval.Context, approval.Options, now)
		}
		s.broadcast(*msg)
		if msg.Type == "approval_detected" {
			s.notifyApprovalPush(id, approval.Sig, msg.Provider, approval.Question, approval.Context)
			s.notifyApprovalOutbound(id, approval.Sig, msg.Provider, approval.Question, approval.Context)
		}
	}
}

// handleDoneSummaryMarker は PTY データから [MANY-AI-CLI-DONE] マーカーを検出し、
// 完了サマリーを履歴と通知へ発行する。外部通知が OFF でも Hub 履歴は残す。
func (s *Server) handleDoneSummaryMarker(id int, data []byte) {
	summaries := extractDoneSummaryTexts(data)
	if len(summaries) == 0 {
		return
	}

	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	// Shell session は AI タスク完了サマリーの対象外
	if !isAIProvider(ses.Provider) {
		s.sessionsMu.Unlock()
		return
	}
	if !ses.lastDoneNotifyAt.IsZero() && now.Sub(ses.lastDoneNotifyAt) < doneNotifyMinInterval {
		s.sessionsMu.Unlock()
		return
	}
	ses.lastDoneNotifyAt = now
	titleName := doneSummaryTitle(ses)
	provider := ses.Provider
	s.sessionsMu.Unlock()
	for _, summary := range summaries {
		s.publishDoneSummary(proto.DoneSummary{SessionID: id, Provider: provider, Title: titleName, Text: summary, At: now.Format(time.RFC3339)})
	}
}

func extractDoneSummaryTexts(data []byte) []string {
	var summaries []string
	remaining := data
	for {
		open := bytes.Index(remaining, doneSummaryMarkerOpen)
		if open < 0 {
			break
		}
		start := open + len(doneSummaryMarkerOpen)
		closeIdx := bytes.Index(remaining[start:], doneSummaryMarkerClose)
		if closeIdx < 0 {
			break
		}
		if summary := strings.TrimSpace(string(remaining[start : start+closeIdx])); summary != "" {
			summaries = append(summaries, summary)
		}
		remaining = remaining[start+closeIdx+len(doneSummaryMarkerClose):]
	}
	return summaries
}

// notifyDonePush は Web Push でタスク完了通知を送信する。
// Web Push 経路は notifyApprovalPush を流用する（同じ Web Push チャンネルを使う）。
func (s *Server) notifyDonePush(summary proto.DoneSummary) {
	// Web Push currently shares the delivery channel and click target with an
	// approval notification. C4's service-worker action templates consume the
	// same title/body shape; no approval token is attached to DONE events.
	s.notifyApprovalPush(summary.SessionID, fmt.Sprintf("done-%d-%d", summary.SessionID, time.Now().UnixNano()), summary.Provider, summary.Text, "")
}

func (s *Server) markNativeApprovalConsumed(m proto.Message) {
	if m.SessionID <= 0 || m.ApprovalSig == "" {
		return
	}
	now := time.Now()
	var clearMsg *proto.Message
	s.sessionsMu.Lock()
	ses := s.sessions[m.SessionID]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	ses.nativeApprovalConsumed = m.ApprovalSig
	ses.nativeApprovalConsumedAt = now
	if ses.nativeApprovalSig == m.ApprovalSig {
		ses.nativeApprovalSig = ""
		ses.nativeApprovalClearMisses = 0
		clearMsg = &proto.Message{
			Type:           "approval_cleared",
			SessionID:      m.SessionID,
			Provider:       ses.Provider,
			ApprovalSig:    m.ApprovalSig,
			ApprovalSource: approvalSourceGoVT,
		}
	}
	s.sessionsMu.Unlock()
	if s.sessionStore != nil {
		s.sessionStore.StoreApprovalConsumed(m.SessionID, m.ApprovalSig, m.SentText, now)
	}
	if clearMsg != nil {
		s.broadcast(*clearMsg)
	}
}

func appendPTYReplay(buf, data []byte) []byte {
	if len(data) == 0 {
		return buf
	}
	if cap(buf) > maxPTYBuf {
		if len(buf) > maxPTYBuf {
			buf = buf[len(buf)-maxPTYBuf:]
		}
		compact := make([]byte, len(buf), maxPTYBuf)
		copy(compact, buf)
		buf = compact
	}
	if len(data) >= maxPTYBuf {
		if cap(buf) < maxPTYBuf {
			buf = make([]byte, maxPTYBuf)
		} else {
			buf = buf[:maxPTYBuf]
		}
		copy(buf, data[len(data)-maxPTYBuf:])
		return buf
	}
	if len(buf)+len(data) <= maxPTYBuf {
		return append(buf, data...)
	}
	keep := maxPTYBuf - len(data)
	if keep > 0 {
		copy(buf, buf[len(buf)-keep:])
		buf = buf[:keep]
	} else {
		buf = buf[:0]
	}
	return append(buf, data...)
}

func ptyChunkContainsAny(data []byte, tokens [][]byte) bool {
	for _, token := range tokens {
		if bytes.Contains(data, token) {
			return true
		}
	}
	return false
}

func nativeApprovalTailSignature(lines []string) string {
	h := fnv.New64a()
	for _, line := range lines {
		_, _ = h.Write([]byte(line))
		_, _ = h.Write([]byte{0})
	}
	return strconv.FormatUint(h.Sum64(), 16)
}

func shouldSuppressNativeApprovalClearMiss(provider string, lines []string) bool {
	if !providerSupportsShortcutApproval(provider) {
		return false
	}
	nonEmpty := 0
	for _, line := range lines {
		if strings.TrimSpace(line) == "" {
			continue
		}
		nonEmpty++
		if nonEmpty > nativeApprovalBlankLineLimit {
			return false
		}
	}
	return true
}

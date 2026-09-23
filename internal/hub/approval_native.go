package hub

// approval_native.go: server.go から分離した「ネイティブ承認検出まわり」の関数群。
//
// C4 (plan_audit_score_s_promotion_2026-07-05.md): server.go 3144 行を関心事別に
// 割るリファクタの第一弾。以下の 9 関数は「Go 側 VT ミラーで検出した provider
// 承認プロンプト」の状態機械 (保留中の記録の開閉 / ClearMisses / DoneSummary
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
	"many-ai-cli/internal/sessionstore"
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
	var closures []*approvalRecordClosure
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
		// 消失の判定はネイティブの記録だけが対象。マーカーの記録は画面の検出では閉じない。
		if ses.pendingApproval.isNative() {
			ses.nativeApprovalClearMisses++
			if ses.nativeApprovalClearMisses >= nativeApprovalClearMissLimit {
				closures = append(closures, closeApprovalRecordLocked(ses, id, approvalCloseVanished, now))
			}
		}
		s.sessionsMu.Unlock()
		s.finishApprovalRecordClosures(closures...)
		return
	}
	candidateKey := approvalCandidateKeyWithContext(ses.Provider, approval.Kind, approval.Question, approval.Context, approval.Options)
	candidateShape := approvalCandidateShapeWithContext(ses.Provider, approval.Kind, approval.Question, approval.Context, approval.Options)
	sourceEpoch, answered := approvalCandidateEpochLocked(ses, candidateKey)
	legacyConsumed := ses.approvalConsumedCandidateKey == "" &&
		ses.nativeApprovalConsumed == approval.Sig && now.Sub(ses.nativeApprovalConsumedAt) < approvalConsumedTTL
	if answered || legacyConsumed {
		s.sessionsMu.Unlock()
		return
	}
	ses.nativeApprovalClearMisses = 0
	provider := ses.Provider
	var opened proto.Message
	var openedActivity *proto.Message
	var openedRecord *approvalRecord
	if !ses.pendingApproval.isCandidate(candidateKey, sourceEpoch) {
		superseded := closeApprovalRecordLocked(ses, id, approvalCloseSuperseded, now)
		superseded.dropActivity()
		closures = append(closures, superseded)
		openedRecord = &approvalRecord{
			CandidateKey:   candidateKey,
			CandidateShape: candidateShape,
			SourceEpoch:    sourceEpoch,
			Sig:            approval.Sig,
			Origin:         approvalRecordOriginNative,
			Source:         approvalSourceGoVT,
			Kind:           approval.Kind,
			Question:       approval.Question,
			Context:        approval.Context,
			Options:        append([]proto.ApprovalOption(nil), approval.Options...),
			Summary:        approval.Summary,
			DetectedAt:     now,
		}
		opened, openedActivity = openApprovalRecordLocked(ses, id, openedRecord)
	}
	s.sessionsMu.Unlock()
	// 古い記録の resolved は新しい記録の INSERT より先に書く（approval_record.go 冒頭）。
	s.finishApprovalRecordClosures(closures...)
	if openedRecord != nil {
		// 自動承認した記録は開いたことを画面へ知らせない。閉じる approval_state だけが届き、
		// 画面は知らない記録の「閉じる」として版番号を進めるだけになる。
		// 選択肢の無い記録（AskUserQuestion の告知）は自動承認の対象にしない。
		if openedRecord.hasOptions() && s.maybeAutoApprove(id, approval) {
			return
		}
		if s.sessionStore != nil {
			s.sessionStore.StoreApprovalDetected(sessionstore.ApprovalDetected{
				LiveSessionID: id,
				Sig:           approval.Sig,
				Source:        approvalSourceGoVT,
				Kind:          approval.Kind,
				Provider:      provider,
				Question:      approval.Question,
				Context:       approval.Context,
				CandidateKey:  candidateKey,
				SourceEpoch:   sourceEpoch,
				Options:       approval.Options,
				DetectedAt:    now,
			})
		}
		s.broadcast(opened)
		if openedActivity != nil {
			s.broadcast(*openedActivity)
		}
		// 承認の通知は記録が開いたときの 1 回だけ（文章の質問も同じ。approval_text_question.go）。
		s.notifyApprovalPush(id, approval.Sig, provider, approval.Question, approval.Context)
		s.notifyApprovalOutbound(id, approval.Sig, provider, approval.Question, approval.Context)
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
	// Keep this separate from lastDoneNotifyAt: a marker can be a duplicate
	// inside the notification interval and therefore be suppressed from the
	// visible stream, but it still proves that this turn was marker-aware.
	ses.doneSummaryMarkerSeen = true
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
	if m.SessionID <= 0 || (m.ApprovalSig == "" && m.ApprovalCandidateKey == "") {
		return
	}
	now := time.Now()
	var closure *approvalRecordClosure
	s.sessionsMu.Lock()
	ses := s.sessions[m.SessionID]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	currentEpoch := ensureApprovalSourceEpochLocked(ses)
	record := ses.pendingApproval
	candidateKey := m.ApprovalCandidateKey
	if candidateKey == "" {
		if m.ApprovalSig != "" && record != nil && record.Sig == m.ApprovalSig {
			candidateKey = record.CandidateKey
		} else {
			candidateKey = m.ApprovalSig // legacy clients only supplied approval_sig
		}
	}
	if m.ApprovalSourceEpoch != 0 && m.ApprovalSourceEpoch != currentEpoch {
		// The answer frame normally follows pty_input. If that input opened the
		// next live boundary before this frame was handled, accept exactly that
		// one-frame handoff only when the candidate is still the active one (or
		// Hub never had an active candidate, as with browser-only fallback).
		previousEpoch := m.ApprovalSourceEpoch+1 == currentEpoch
		activeSameCandidate := record.isCandidate(candidateKey, m.ApprovalSourceEpoch)
		if !previousEpoch || (approvalCandidateActiveLocked(ses) && !activeSameCandidate) {
			// A delayed answer from an older prompt generation must not consume a
			// newly displayed prompt with the same question.
			s.sessionsMu.Unlock()
			return
		}
	}
	sourceEpoch := m.ApprovalSourceEpoch
	if sourceEpoch == 0 || sourceEpoch == currentEpoch {
		sourceEpoch = markApprovalConsumedLocked(ses, candidateKey, m.ApprovalSig)
	} else {
		sourceEpoch = markApprovalConsumedAtEpochLocked(ses, candidateKey, m.ApprovalSig, sourceEpoch)
	}
	// 画面は記録の sig と同一性（candidate_key + source_epoch）をそのまま返す。ネイティブは
	// sig の一致でも引き当てる（approval_sig だけを送る古い画面のため。上の candidateKey の補完）。
	// マーカーは同一性だけで引き当てる（古い画面はマーカーの approval_sig に選択肢から計算した
	// 別の値を載せていた）。
	nativeMatched := record.isNative() && (record.Sig == m.ApprovalSig || record.isCandidate(candidateKey, sourceEpoch))
	markerMatched := record.isMarker() && record.isCandidate(candidateKey, sourceEpoch)
	if nativeMatched || markerMatched {
		// 台帳の行は記録の Sig（Hub がブロック本文・選択肢から取った値）で引く
		// （finishApprovalRecordClosures。bugfix_approval-panel-lost-after-transcript-marker_2026-09-08.md）。
		closure = closeApprovalRecordLocked(ses, m.SessionID, approvalCloseAnswered, now)
	}
	s.sessionsMu.Unlock()
	if closure != nil {
		closure.answer = m.SentText
		s.finishApprovalRecordClosures(closure)
		return
	}
	// 記録と一致しない回答（記録が既に閉じている）でも、台帳に同じ sig の行があれば resolved にする。
	if s.sessionStore != nil && m.ApprovalSig != "" {
		s.sessionStore.StoreApprovalConsumed(m.SessionID, m.ApprovalSig, m.SentText, now)
	}
}

// evaluateReplayApproval performs the single approval evaluation allowed at
// the end of a replay boundary. PTY history has already been restored into the
// VT mirror before this function is called; no replay chunk is broadcast as a
// live approval observation.
func (s *Server) evaluateReplayApproval(id int) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.vt == nil || !sessionApprovalDetectionEligible(ses) || now.Before(ses.vtResizeDebounceUntil) {
		s.sessionsMu.Unlock()
		return
	}
	provider := ses.Provider
	// トランスクリプトが供給元のセッションでは、ここでも VT からマーカーを立てない。
	// reattach ではパーサ状態を作り直すので、次の poll の prime が最後の assistant
	// メッセージを見て、未回答の質問ならそこから配信し直す
	// （approval_marker_transcript.go の scanTranscriptApprovalMarkers）。
	var marker *approvalMarkerBlock
	var question *textQuestion
	screen := ses.vt.TailLines(vtTailLinesForApproval)
	if !approvalMarkerSourceIsTranscriptLocked(ses) {
		marker = extractApprovalMarkerBlockFromVT(ses.vt)
		// 文章の質問は出力が落ち着いているときだけ見る。まだ動いているなら、落ち着いた時点で
		// openTextQuestionsOnOutputIdle が見る（approval_text_question.go の「出力が落ち着いてから開く」節）。
		if marker == nil && ses.Activity.OutputIdle {
			question = detectTextQuestion(screen)
		}
	}
	approval := s.detectScreenApproval(provider, screen)
	s.sessionsMu.Unlock()
	if marker != nil {
		s.maybeBroadcastApprovalMarker(id, marker, now)
	}
	s.openTextQuestion(id, question, now, approvalSourceGoVT)
	s.handleNativeApprovalDetection(id, approval)
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

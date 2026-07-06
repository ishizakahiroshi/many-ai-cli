package hub

// input_gate.go: server.go から分離した「ユーザー入力ゲート・保留キュー」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第二弾。以下の 10 関数は「pty_input を wrapper へ届ける経路 + 初期プロンプト
// 注入ゲート中の保留 + 未接続時の pending キュー + bracketed-paste 二段送信 + 直列化」
// を扱う一塊で、他の関心事から明確に分離できる。挙動は移動前と完全に同一・全て
// package-private・呼び出し元は変更なし。

import (
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

// handleInput は pty_input メッセージを wrapper へ届ける。
// Enter 確定時はセッション概要（FirstMessage/LastMessage）を更新し、
// ユーザーターン境界マーカーを ptyBuf に注入する。
func (s *Server) handleInput(m proto.Message) {
	s.sessionsMu.Lock()
	wc := s.wrappers[m.SessionID]
	ses := s.sessions[m.SessionID]
	combined := m.Text
	var firstMsgBroadcast *proto.Message
	var injectMarker bool
	if ses != nil && strings.HasSuffix(m.Text, "\r") {
		text := strings.TrimRight(m.Text, "\r\n")
		if text == "/clear" {
			// /clear でセッション概要をリセット（次の入力が新しい概要になる）
			ses.FirstMessage = ""
			ses.LastMessage = ""
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt}
			firstMsgBroadcast = &msg
		} else if text != "" {
			maskedText := sessionlog.MaskSecrets(text)
			if ses.FirstMessage == "" {
				ses.FirstMessage = maskedText
			}
			// 数字のみ（選択肢番号）は LastMessage を更新しない
			if !isDigitsOnly(text) {
				ses.LastMessage = maskedText
			}
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt, FirstMessage: ses.FirstMessage, LastMessage: ses.LastMessage}
			firstMsgBroadcast = &msg
			// ユーザーターン境界マーカーを ptyBuf に注入する
			marker := []byte(chatHistoryUserTurnMarker)
			ses.ptyBuf = appendPTYReplay(ses.ptyBuf, marker)
			injectMarker = true
		}
	}
	s.sessionsMu.Unlock()
	if injectMarker {
		s.broadcast(proto.Message{Type: "pty_data", SessionID: m.SessionID, Data: []byte(chatHistoryUserTurnMarker)})
	}
	s.submitInput(wc, m.SessionID, combined)
	s.writeHistory(m.SessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "user_input",
		"session_id": m.SessionID,
		"text":       sessionlog.MaskSecrets(m.Text),
	})
	if firstMsgBroadcast != nil {
		s.broadcast(*firstMsgBroadcast)
	}
}

func splitBracketedPasteSubmit(text string) (first string, delayed string) {
	if !strings.HasSuffix(text, bracketedPasteEnd+"\r") {
		return text, ""
	}
	return strings.TrimSuffix(text, "\r"), "\r"
}

// maxPendingInputPerSession は 1 セッションあたりの保留入力の上限。
// wrapper が長時間戻らないケースで無制限に溜まるのを防ぐ。超過時は古い方から捨てる。
const maxPendingInputPerSession = 100

// initialInjectGateMaxAge は初期プロンプト注入ゲートの生存上限。注入経路の事故
// （spawn タイムアウト後の遅延登録等）で clearInitialInjectGate が呼ばれないまま
// ゲートが張り付いても、この時間を超えたら入力保留をやめて通常送信に戻す保険。
const initialInjectGateMaxAge = 90 * time.Second

// sessionInjectGated は初期プロンプト注入ゲートが有効かを返す。sessionsMu 保持下で呼ぶ。
func sessionInjectGated(ses *session, now time.Time) bool {
	return ses.initialInjectPending && now.Sub(ses.initialInjectGateAt) < initialInjectGateMaxAge
}

// submitInput はユーザー入力を wrapper へ届ける。wrapper 未接続・送信失敗時は
// 入力を順序保持でバッファし、wrapper の (再)接続時に flushPendingInput が自動再送する
// （= 黙って捨てない）。既に保留中の入力があるセッションでは、新規入力を直送せず
// 末尾へ積んで順序を保つ。
//
// per-session inputMu (#18) により、複数 UI が同一セッションへ同時に入力しても
// hasPending チェック〜trySendInput（50ms sleep 含む bracketd-paste 二段送信）が
// 直列化され、bracketed-paste 本文と確定 CR のインターリーブが起きない。
// sessionsMu は inputMu の外側でのみ取得し、50ms sleep 中に保持しない。
func (s *Server) submitInput(wc *wrapperConn, sessionID int, combined string) {
	s.submitInputWithGate(wc, sessionID, combined, false)
}

// submitInputWithGate は submitInput の実体。bypassGate=true は初期プロンプト注入
// （injectInitialPrompt）専用で、注入ゲート中でも wrapper へ直接送る。
func (s *Server) submitInputWithGate(wc *wrapperConn, sessionID int, combined string, bypassGate bool) {
	// session ポインタを短期間だけ sessionsMu で取得する。
	// session が既に削除済みの場合は nil になるので早期リターンする。
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	s.sessionsMu.Unlock()

	if ses == nil {
		// セッションが既に終了している場合は入力を捨てる（黙って失わない挙動は
		// 存在するセッションへの入力に限る）。
		return
	}

	// per-session 入力直列化ロック: hasPending チェック〜trySendInput 完了まで保持。
	// 複数 UI が同時にこの関数を呼んでも、同一 sessionID に対しては 1 件ずつ処理される。
	ses.inputMu.Lock()
	defer ses.inputMu.Unlock()

	s.sessionsMu.Lock()
	gated := !bypassGate && sessionInjectGated(ses, time.Now())
	hasPending := len(s.pendingInput[sessionID]) > 0
	if gated || hasPending {
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], combined)
	}
	s.sessionsMu.Unlock()
	if gated || hasPending {
		s.notifyInputDeferred(sessionID)
		return
	}
	if rem := s.trySendInput(wc, sessionID, combined); rem != "" {
		s.sessionsMu.Lock()
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], rem)
		s.sessionsMu.Unlock()
		s.notifyInputDeferred(sessionID)
	}
}

// trySendInput は combined を wrapper へ送る。届けられなかった残り（未送信部分）を返す
// （"" = 全て送信済み）。bracketed-paste の確定 \r は別書き込み + 50ms 遅延で送る従来挙動を保つ。
// first まで送れて delayed(\r) だけ失敗した場合は \r のみを残りとして返し、本文の二重送信を避ける。
func (s *Server) trySendInput(wc *wrapperConn, sessionID int, combined string) (remaining string) {
	if wc == nil {
		s.logger.Warn("pty_input deferred: no wrapper connected", "session_id", sessionID)
		return combined
	}
	first, delayed := splitBracketedPasteSubmit(combined)
	if err := wc.send(proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(first)}); err != nil {
		s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "first", "err", err)
		return combined
	}
	if delayed != "" {
		time.Sleep(bracketedPasteSubmitDelay)
		if err := wc.send(proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(delayed)}); err != nil {
			s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "delayed", "err", err)
			return delayed
		}
	}
	return ""
}

// flushPendingInput は wrapper の (再)接続後に保留入力を順番に再送する。
// trySendInput が遅延 sleep しうるため goroutine で呼ぶ前提。再送に失敗した場合は
// 残りを先頭へ戻し、次の接続でリトライする。
// per-session inputMu (#18) を保持して実行するため、フラッシュ中に submitInput が
// 割り込んで入力順序が乱れることはない。
func (s *Server) flushPendingInput(sessionID int) {
	// session ポインタを短期間だけ sessionsMu で取得する。
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	s.sessionsMu.Unlock()
	if ses == nil {
		return
	}

	// per-session 入力直列化ロック: pending ドレイン中に submitInput が割り込まないよう保持。
	ses.inputMu.Lock()
	defer ses.inputMu.Unlock()

	s.sessionsMu.Lock()
	if sessionInjectGated(ses, time.Now()) {
		// 初期プロンプト注入ゲート中は保留したまま何もしない（wrapper 再接続時の
		// フラッシュで注入前にユーザー入力が流れるのを防ぐ）。ゲート解除時に
		// clearInitialInjectGate が再度フラッシュする。
		s.sessionsMu.Unlock()
		return
	}
	pending := s.pendingInput[sessionID]
	delete(s.pendingInput, sessionID)
	wc := s.wrappers[sessionID]
	s.sessionsMu.Unlock()
	if len(pending) == 0 {
		return
	}
	if wc == nil {
		s.requeuePendingInput(sessionID, pending)
		return
	}
	var remainder []string
	for i, combined := range pending {
		if rem := s.trySendInput(wc, sessionID, combined); rem != "" {
			remainder = append(remainder, rem)
			remainder = append(remainder, pending[i+1:]...)
			break
		}
	}
	if len(remainder) > 0 {
		s.requeuePendingInput(sessionID, remainder)
		return
	}
	s.logger.Info("flushed deferred pty_input", "session_id", sessionID, "count", len(pending))
}

// requeuePendingInput は再送できなかった残りを保留キューの先頭へ戻す
// （フラッシュ中に新規到着した入力は後ろに残す）。
func (s *Server) requeuePendingInput(sessionID int, queue []string) {
	s.sessionsMu.Lock()
	if existing := s.pendingInput[sessionID]; len(existing) > 0 {
		queue = append(queue, existing...)
	}
	if len(queue) > maxPendingInputPerSession {
		queue = queue[len(queue)-maxPendingInputPerSession:]
	}
	s.pendingInput[sessionID] = queue
	s.sessionsMu.Unlock()
}

// appendPendingInput は保留キューへ 1 件積み、上限超過分を古い方から捨てる。
func appendPendingInput(q []string, item string) []string {
	q = append(q, item)
	if len(q) > maxPendingInputPerSession {
		q = q[len(q)-maxPendingInputPerSession:]
	}
	return q
}

// notifyInputDeferred は UI へ「入力を保留した（wrapper 未接続/送信失敗）」を通知する。
func (s *Server) notifyInputDeferred(sessionID int) {
	s.broadcast(proto.Message{Type: "input_deferred", SessionID: sessionID})
}

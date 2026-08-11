package hub

// input_gate.go: server.go から分離した「ユーザー入力ゲート・保留キュー」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第二弾。以下の 10 関数は「pty_input を wrapper へ届ける経路 + 初期プロンプト
// 注入ゲート中の保留 + 未接続時の pending キュー + bracketed-paste 二段送信 + 直列化」
// を扱う一塊で、他の関心事から明確に分離できる。挙動は移動前と完全に同一・全て
// package-private・呼び出し元は変更なし。

import (
	"fmt"
	"sort"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/sessionstore"
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
	var userTurnEpoch uint64
	var autoTitleMeta *sessionstore.SessionCardMeta
	// チャット本文はブラケットペースト包み（... \x1b[201~）+ 確定 \r 別送で届くため、
	// 末尾 \r だけでなくペースト終端もユーザーターンの確定として扱う。従来の
	// 「末尾 \r のみ」判定では、ペースト送信のセッション概要（FirstMessage 等）と
	// ターン境界マーカーが一切更新されなかった（複数行送信の既存ギャップ。単一行も
	// ペースト経路に統一した 2026-07-11 以降は全チャット送信が該当するため必須）。
	// メタデータ用テキストはペーストマーカーを剥がして評価する。後続の確定 \r は
	// 剥がした後に空文字となり、二重更新・二重マーカーにはならない。
	if ses != nil && (strings.HasSuffix(m.Text, "\r") || strings.HasSuffix(m.Text, bracketedPasteEnd)) {
		text := strings.TrimRight(m.Text, "\r\n")
		text = strings.ReplaceAll(text, bracketedPasteStart, "")
		text = strings.ReplaceAll(text, bracketedPasteEnd, "")
		if text == "/clear" {
			// /clear でセッション概要をリセット（次の入力が新しい概要になる）
			ses.FirstMessage = ""
			ses.LastMessage = ""
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt}
			firstMsgBroadcast = &msg
		} else if text != "" {
			// A confirmed live user turn is the explicit prompt boundary. This is
			// deliberately not inferred from replay or a VT reflow. The Enter that
			// answers the currently visible approval arrives before the UI's
			// approval_consumed frame; leave that candidate in the current epoch so
			// the consumed frame can settle it. If Hub has no active candidate (the
			// browser-only fallback path), this same boundary still advances the
			// epoch so a repeated question is not suppressed forever.
			if !approvalCandidateActiveLocked(ses) {
				markApprovalUserTurnBoundaryLocked(ses)
			}
			maskedText := sessionlog.MaskSecrets(text)
			if ses.FirstMessage == "" {
				ses.FirstMessage = maskedText
				// 最初の依頼を短い自動タイトルにする。ラベルは wrapper /
				// orchestration の識別子も兼ねるため書き換えず、UI では手動
				// label を優先して AutoTitle をフォールバックとして使う。
				if ses.AutoTitle == "" {
					ses.AutoTitle = normalizeSessionMetaText(maskedText, 40)
					meta := sessionStoreMeta(ses)
					autoTitleMeta = &meta
				}
			}
			// 数字のみ（選択肢番号）は LastMessage を更新しない
			if !isDigitsOnly(text) {
				ses.LastMessage = maskedText
			}
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt, FirstMessage: ses.FirstMessage, LastMessage: ses.LastMessage, SessionMeta: sessionMetaFor(ses)}
			firstMsgBroadcast = &msg
			// ユーザーターン境界マーカーを ptyBuf に注入する
			marker := []byte(chatHistoryUserTurnMarker)
			ses.ptyBuf = appendPTYReplay(ses.ptyBuf, marker)
			userTurnEpoch = ensureApprovalSourceEpochLocked(ses)
			injectMarker = true
		}
	}
	s.sessionsMu.Unlock()
	if injectMarker {
		// Review Phase 2: AI へ入力を渡す前の作業ツリーを、このターンの開始点として
		// 記録する。既に開始点がある場合（承認回答などターン途中の追加入力）は
		// captureGitTurnStart 側で維持し、途中までの編集を取りこぼさない。
		// 入力経路の診断。captureGitTurnStart は
		// 意図的に同期（git_turns.go）で、git add -A を含む 4 本のコマンドを
		// uiLoop の中で走らせる。ここが長いと、その間 同じ UI 接続から届く
		// 確定 \r を Hub が受信できない。実測値が無いと切り分けられない。
		gitCaptureStart := time.Now()
		s.captureGitTurnStart(m.SessionID)
		s.logger.Info("input_trace",
			"stage", "git_capture",
			"session_id", m.SessionID,
			"elapsed_ms", time.Since(gitCaptureStart).Milliseconds())
		s.broadcast(proto.Message{Type: "pty_data", SessionID: m.SessionID, Data: []byte(chatHistoryUserTurnMarker)})
		// ターン完了カードの自動消去用。ターン境界マーカーは ptyBuf 経由で attach
		// リプレイにも再配信されるため信号に使えない。State("running") は PTY 出力
		// 再開の表示ラベルで resize 再描画等でも遷移する（session_activity.go の警告）。
		// ここ（確定ユーザー入力の provider 送達）だけがライブ限定の正確な境界。
		s.broadcast(proto.Message{Type: "user_turn_started", SessionID: m.SessionID, ApprovalSourceEpoch: userTurnEpoch})
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
	if autoTitleMeta != nil && s.sessionStore != nil {
		// AutoTitle は first message の確定時だけ変化する。入力ホットパスで
		// SQLite を待たないよう既存のメッセージ更新と同じ軽量更新に留める。
		_ = s.sessionStore.UpdateSessionCardMeta(m.SessionID, *autoTitleMeta)
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

// maxInflightInputPerSession bounds inputs that have been sent to a wrapper
// but are waiting for pty_input_ack. Keep the same bound as pendingInput so a
// disconnected wrapper cannot retain unbounded user input in memory.
const maxInflightInputPerSession = maxPendingInputPerSession

type inflightInput struct {
	data string
	conn *wrapperConn
}

// pendingFrame は再送待ちの 1 フレーム。seq を保持するのが要点で、これにより
// wrapper 側が「既に PTY へ書いた分の再送」と判定して二重書き込みを避けられる。
// 新しい seq を振り直すと重複判定が効かず、確定 \r が 2 回入りうる。
type pendingFrame struct {
	seq  int64
	data string
}

// reserveInflightInput records a frame before it is written to the wrapper.
// Recording first closes the race where the wrapper disconnects immediately
// after Hub's websocket write returns.
func (s *Server) reserveInflightInput(wc *wrapperConn, sessionID int, data string) int64 {
	if wc == nil || data == "" {
		return 0
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return 0
	}
	if ses.inflightInput == nil {
		ses.inflightInput = map[int64]inflightInput{}
	}
	for {
		ses.inputSeq++
		if ses.inputSeq <= 0 {
			ses.inputSeq = 1
		}
		if _, exists := ses.inflightInput[ses.inputSeq]; !exists {
			break
		}
	}
	seq := ses.inputSeq
	ses.inflightInput[seq] = inflightInput{data: data, conn: wc}
	for len(ses.inflightInput) > maxInflightInputPerSession {
		var oldest int64
		for candidate := range ses.inflightInput {
			if oldest == 0 || candidate < oldest {
				oldest = candidate
			}
		}
		delete(ses.inflightInput, oldest)
	}
	return seq
}

func (s *Server) releaseInflightInput(wc *wrapperConn, sessionID int, seq int64) {
	if wc == nil || seq <= 0 {
		return
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return
	}
	if item, ok := ses.inflightInput[seq]; ok && item.conn == wc {
		delete(ses.inflightInput, seq)
	}
}

// sendPTYInputFrame attaches a sequence number to one Hub-to-wrapper frame.
// Empty frames retain the legacy wire behavior and are not tracked because the
// wrapper intentionally does not acknowledge them.
func (s *Server) sendPTYInputFrame(wc *wrapperConn, sessionID int, data string) error {
	if wc == nil {
		return fmt.Errorf("wrapper not connected")
	}
	seq := s.reserveInflightInput(wc, sessionID, data)
	if data != "" && seq == 0 {
		return fmt.Errorf("session %d is not registered", sessionID)
	}
	m := proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(data), InputSeq: seq}
	if err := wc.send(m); err != nil {
		s.releaseInflightInput(wc, sessionID, seq)
		return err
	}
	return nil
}

// sendPTYInputFrameWithSeq は未 ack のまま切断されたフレームを、元の seq のまま
// 送り直す。seq を振り直さないので、既に PTY へ入っていた分は wrapper 側が
// 握り潰して ack だけ返し、二重書き込みにならない。
func (s *Server) sendPTYInputFrameWithSeq(wc *wrapperConn, sessionID int, data string, seq int64) error {
	if wc == nil {
		return fmt.Errorf("wrapper not connected")
	}
	if !s.readmitInflightInput(wc, sessionID, seq, data) {
		return fmt.Errorf("session %d is not registered", sessionID)
	}
	m := proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(data), InputSeq: seq}
	if err := wc.send(m); err != nil {
		s.releaseInflightInput(wc, sessionID, seq)
		return err
	}
	return nil
}

// readmitInflightInput は再送するフレームを、元の seq のまま新しい接続の
// in-flight として登録し直す。ack が返れば消え、また切れれば再び再送キューへ戻る。
func (s *Server) readmitInflightInput(wc *wrapperConn, sessionID int, seq int64, data string) bool {
	if wc == nil || seq <= 0 {
		return false
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return false
	}
	if ses.inflightInput == nil {
		ses.inflightInput = map[int64]inflightInput{}
	}
	ses.inflightInput[seq] = inflightInput{data: data, conn: wc}
	return true
}

// handlePTYInputAck removes the matching in-flight frame. Marking the
// connection as ack-capable is intentionally independent of whether the seq
// is still present: a late/duplicate ack is still evidence of a new wrapper.
func (s *Server) handlePTYInputAck(wc *wrapperConn, sessionID int, seq int64) {
	if wc == nil {
		return
	}
	wc.inputAckSeen.Store(true)
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return
	}
	// 接続単位のフラグだけでは足りない。reattach のたびに wrapperConn は作り直され、
	// 「再接続直後に 1 件も ack を受けないまま再び切れる」窓では旧 wrapper と
	// 区別できず、再送されずに入力が消える。セッション単位でも記憶しておく。
	ses.inputAckCapable = true
	if seq <= 0 {
		return
	}
	if item, ok := ses.inflightInput[seq]; ok && item.conn == wc {
		delete(ses.inflightInput, seq)
	}
}

// deferInflightForResendLocked は wc に紐づく未 ack のフレームを再送キューへ移す。
// ack を返す wrapper だと分かっているセッションだけが対象で、ack を一度も返さない
// 旧 wrapper では従来どおり再送しない（二重書き込みを避けるため）。
//
// pendingInput ではなく専用の resendInput へ積むのは、元の seq を保ったまま
// 送り直す必要があるから。pendingInput は []string で seq を運べず、再送時に
// 新しい seq が振られてしまい、wrapper 側の重複判定が効かなくなる。
func (s *Server) deferInflightForResendLocked(sessionID int, wc *wrapperConn) (count int, minSeq int64, maxSeq int64) {
	ses := s.sessions[sessionID]
	if ses == nil || len(ses.inflightInput) == 0 || wc == nil {
		return 0, 0, 0
	}
	items := make([]pendingFrame, 0, len(ses.inflightInput))
	for seq, item := range ses.inflightInput {
		if item.conn != wc {
			continue
		}
		items = append(items, pendingFrame{seq: seq, data: item.data})
		delete(ses.inflightInput, seq)
	}
	if len(items) == 0 || !(wc.inputAckSeen.Load() || ses.inputAckCapable) {
		return 0, 0, 0
	}
	sort.Slice(items, func(i, j int) bool { return items[i].seq < items[j].seq })
	ses.resendInput = mergeResendFrames(ses.resendInput, items)
	return len(items), items[0].seq, items[len(items)-1].seq
}

// mergeResendFrames は再送キューを seq 昇順で束ね、上限を超えた古い方から捨てる。
func mergeResendFrames(existing, incoming []pendingFrame) []pendingFrame {
	queue := make([]pendingFrame, 0, len(existing)+len(incoming))
	queue = append(queue, existing...)
	queue = append(queue, incoming...)
	sort.Slice(queue, func(i, j int) bool { return queue[i].seq < queue[j].seq })
	if len(queue) > maxInflightInputPerSession {
		queue = queue[len(queue)-maxInflightInputPerSession:]
	}
	return queue
}

// requeueResendInput は送り直せなかった再送フレームをキューへ戻す。
func (s *Server) requeueResendInput(sessionID int, frames []pendingFrame) {
	if len(frames) == 0 {
		return
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return
	}
	ses.resendInput = mergeResendFrames(ses.resendInput, frames)
}

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
	pendingLen := len(s.pendingInput[sessionID])
	hasPending := pendingLen > 0
	if gated || hasPending {
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], combined)
	}
	s.sessionsMu.Unlock()
	// 入力経路の診断は内容を記録せず、状態だけを残す。保留された事実は既存 WARN で
	// 分かるが、「保留されずに直送された」側が無記録だったため、本文だけ通って
	// 確定 \r が来ていないのか、両方通ったのかが判別できなかった。
	s.logger.Info("input_trace",
		"stage", "gate",
		"session_id", sessionID,
		"bytes", len(combined),
		"gated", gated,
		"has_pending", hasPending,
		"pending_len", pendingLen)
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
// （"" = 全て送信済み）。各フレームは送信前に in-flight へ記録し、wrapper の ack が
// 届くまで保持する。bracketed-paste の確定 \r は別書き込み + 50ms 遅延で送る従来挙動を保つ。
// first まで送れて delayed(\r) だけ失敗した場合は \r のみを残りとして返し、本文の二重送信を避ける。
func (s *Server) trySendInput(wc *wrapperConn, sessionID int, combined string) (remaining string) {
	if wc == nil {
		s.logger.Warn("pty_input deferred: no wrapper connected", "session_id", sessionID)
		return combined
	}
	first, delayed := splitBracketedPasteSubmit(combined)
	sendStart := time.Now()
	if err := s.sendPTYInputFrame(wc, sessionID, first); err != nil {
		s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "first", "err", err)
		return combined
	}
	if delayed != "" {
		time.Sleep(bracketedPasteSubmitDelay)
		if err := s.sendPTYInputFrame(wc, sessionID, delayed); err != nil {
			s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "delayed", "err", err)
			return delayed
		}
	}
	// 入力経路の診断は内容を記録せず、送信状態だけを残す。wc.send は wrapper からの ack が
	// 無く write deadline も持たないため、err=nil でも wrapper に届いた保証は無い。
	// 送信状態と経過時間だけを残し、入力内容は記録しない。
	s.logger.Info("input_trace",
		"stage", "sent",
		"session_id", sessionID,
		"bytes", len(combined),
		"split", delayed != "",
		"elapsed_ms", time.Since(sendStart).Milliseconds(),
		"ts_ns", time.Now().UnixNano())
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
	resend := ses.resendInput
	ses.resendInput = nil
	wc := s.wrappers[sessionID]
	s.sessionsMu.Unlock()
	if len(pending) == 0 && len(resend) == 0 {
		return
	}
	if wc == nil {
		s.requeueResendInput(sessionID, resend)
		s.requeuePendingInput(sessionID, pending)
		return
	}
	// 未 ack 分を先に、元の seq のまま送り直す。既に PTY へ入っていた分は
	// wrapper 側が seq で重複と判定して握り潰し、ack だけ返す。
	for i, frame := range resend {
		if err := s.sendPTYInputFrameWithSeq(wc, sessionID, frame.data, frame.seq); err != nil {
			s.logger.Warn("pty_input resend failed", "session_id", sessionID, "input_seq", frame.seq, "err", err)
			s.requeueResendInput(sessionID, resend[i:])
			s.requeuePendingInput(sessionID, pending)
			return
		}
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
	s.logger.Info("flushed deferred pty_input", "session_id", sessionID, "count", len(pending), "resent", len(resend))
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

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

// enqueueInputWork はセッション単位の FIFO へ入力処理を積む。呼び出し元
// （uiLoop の pty_input 分岐）は即座に戻り、次の WebSocket メッセージを読める。
//
// なぜ worker goroutine + channel ではなくバトン渡しなのか: worker 方式は
// 「いつ止めるか」の寿命管理が要り、セッション削除の取りこぼしがそのまま
// goroutine と入力の滞留になる。ここでは各入力が「直前の入力の完了 channel」を
// 待つだけなので、鎖が尽きれば goroutine は自然に消える。生成順 = 実行順が
// goroutine の起動順に依存しないのも要点で、待ち先は enqueue 時点で確定する。
func (s *Server) enqueueInputWork(sessionID int, fn func()) {
	s.inputChainMu.Lock()
	if s.inputChain == nil {
		s.inputChain = map[int]chan struct{}{}
	}
	prev := s.inputChain[sessionID]
	done := make(chan struct{})
	s.inputChain[sessionID] = done
	s.inputChainMu.Unlock()

	go func() {
		if prev != nil {
			<-prev
		}
		defer func() {
			close(done)
			// 自分が最後尾のままなら鎖を畳む（セッションが消えても残骸を持たない）。
			s.inputChainMu.Lock()
			if s.inputChain[sessionID] == done {
				delete(s.inputChain, sessionID)
			}
			s.inputChainMu.Unlock()
		}()
		fn()
	}()
}

// handleInput は pty_input メッセージを wrapper へ届ける。
// Enter 確定時はセッション概要（FirstMessage/LastMessage）を更新し、
// ユーザーターン境界マーカーを ptyBuf に注入する。
func (s *Server) handleInput(m proto.Message) {
	s.sessionsMu.Lock()
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
					ses.AutoTitle = autoTitleFromInput(maskedText)
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
		s.captureGitTurnStart(m.SessionID)
		s.broadcast(proto.Message{Type: "pty_data", SessionID: m.SessionID, Data: []byte(chatHistoryUserTurnMarker)})
		// ターン完了カードの自動消去用。ターン境界マーカーは ptyBuf 経由で attach
		// リプレイにも再配信されるため信号に使えない。State("running") は PTY 出力
		// 再開の表示ラベルで resize 再描画等でも遷移する（session_activity.go の警告）。
		// ここ（確定ユーザー入力の provider 送達）だけがライブ限定の正確な境界。
		s.broadcast(proto.Message{Type: "user_turn_started", SessionID: m.SessionID, ApprovalSourceEpoch: userTurnEpoch})
	}
	s.submitInput(m.SessionID, combined)
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
// hasPending チェック〜trySendInput（確定 CR の静止待ちを含む bracketd-paste
// 二段送信）が直列化され、bracketed-paste 本文と確定 CR のインターリーブが
// 起きない。sessionsMu は inputMu の外側でのみ取得し、待機中に保持しない。
func (s *Server) submitInput(sessionID int, combined string) {
	s.submitInputWithGate(sessionID, combined, false)
}

// submitInputWithGate は submitInput の実体。bypassGate=true は初期プロンプト注入
// （injectInitialPrompt）専用で、注入ゲート中でも wrapper へ直接送る。
func (s *Server) submitInputWithGate(sessionID int, combined string, bypassGate bool) {
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
	if s.sessions[sessionID] == nil {
		s.sessionsMu.Unlock()
		return
	}
	// Keep the trace state from the current registration. trySendInput resolves
	// the wrapper again at the actual delivery boundary below.
	wrapperConnected := s.wrappers[sessionID] != nil
	pendingLen := len(s.pendingInput[sessionID])
	hasPending := pendingLen > 0
	if gated || hasPending {
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], combined)
	}
	s.sessionsMu.Unlock()
	// 記録点は sessionsMu の外で呼ぶ。sink が cfgMu を取るため、2 つのロックを
	// 同時に保持しないという Server の規約を守る。
	s.probe("input.gate", "session_id", sessionID,
		"bytes", len(combined), "gated", gated, "has_pending", hasPending, "pending_len", pendingLen)
	if gated || hasPending {
		s.notifyInputDeferred(sessionID)
		return
	}
	rem := s.trySendInput(sessionID, combined)
	s.probe("input.sent", "session_id", sessionID,
		"bytes", len(combined), "remaining", len(rem), "wrapper_connected", wrapperConnected)
	if rem != "" {
		s.sessionsMu.Lock()
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], rem)
		s.sessionsMu.Unlock()
		s.notifyInputDeferred(sessionID)
	}
}

// trySendInput は combined を wrapper へ送る。届けられなかった残り（未送信部分）を返す
// （"" = 全て送信済み）。各フレームは送信前に in-flight へ記録し、wrapper の ack が
// 届くまで保持する。bracketed-paste の確定 \r は別書き込みで送り、送出タイミングは
// 固定遅延ではなく PTY 出力の静止待ちで決める（waitForSubmitEnterSettle）。
// first まで送れて delayed(\r) だけ失敗した場合は \r のみを残りとして返し、本文の二重送信を避ける。
func (s *Server) trySendInput(sessionID int, combined string) (remaining string) {
	// Reattach can replace the wrapper while the caller is waiting. Resolve the
	// registration immediately before creating the wire frame.
	wc := s.currentWrapperForInput(sessionID)
	return s.trySendInputToWrapper(sessionID, wc, combined)
}

// trySendInputToWrapper sends to a wrapper that was validated by a caller
// holding the session input lock. The current-registration checks prevent a
// reattach that happened while waiting from receiving an approval answer meant
// for the previous connection.
func (s *Server) trySendInputToWrapper(sessionID int, wc *wrapperConn, combined string) (remaining string) {
	if wc == nil {
		s.logger.Warn("pty_input deferred: no wrapper connected", "session_id", sessionID)
		return combined
	}
	if s.currentWrapperForInput(sessionID) != wc {
		return combined
	}
	first, delayed := splitBracketedPasteSubmit(combined)
	// 直前がブラケットペースト本文で、今回が確定 CR 単体なら、UI が 2 通に分けて
	// 送ってきた同じ 1 回の送信とみなす。1 通で来た場合（下の delayed 分岐）と同じく
	// 出力静止を待ってから撃ち、動き出さなければ 1 回だけ再送する。
	// この分岐が無いと、本文の配送が遅れた分だけ CR が本文へ密着し、内側 CLI が
	// ペースト取り込み中に CR を吸収して送信が成立しない。
	awaiting := s.takeAwaitingSubmitEnter(sessionID)
	if awaiting && delayed == "" && first == "\r" {
		s.waitForSubmitEnterSettle(sessionID)
		if s.currentWrapperForInput(sessionID) != wc {
			// 送れずに差し戻すので印も戻す。再送時にも確定 CR として扱えるようにする。
			s.setAwaitingSubmitEnter(sessionID)
			return combined
		}
		if err := s.sendPTYInputFrame(wc, sessionID, first); err != nil {
			s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "split_enter", "err", err)
			s.setAwaitingSubmitEnter(sessionID)
			return combined
		}
		s.confirmOrResendSubmitEnter(sessionID, wc, first)
		return ""
	}
	if err := s.sendPTYInputFrame(wc, sessionID, first); err != nil {
		s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "first", "err", err)
		return combined
	}
	if delayed == "" && strings.HasSuffix(first, bracketedPasteEnd) {
		// 次に来る 1 バイトの CR を「この本文の確定」として扱う印。
		s.setAwaitingSubmitEnter(sessionID)
	}
	if delayed != "" {
		// 固定遅延では取り込み・再描画の最中に撃ってしまい、CLI が確定 CR を
		// 吸収する。出力が静止するまで待ってから 1 回だけ撃つ（C1）。
		s.waitForSubmitEnterSettle(sessionID)
		if s.currentWrapperForInput(sessionID) != wc {
			return delayed
		}
		if err := s.sendPTYInputFrame(wc, sessionID, delayed); err != nil {
			s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "delayed", "err", err)
			return delayed
		}
		// 送れたことは確定した証拠にならない。動き出したかを見て、
		// 動いていなければ 1 回だけ再送する（C2）。
		s.confirmOrResendSubmitEnter(sessionID, wc, delayed)
	}
	// wc.send は wrapper からの ack が無く write deadline も持たないため、err=nil でも
	// wrapper に届いた保証は無い。到達確認が要る経路は inputAckCapable 側で扱う。
	return ""
}

// setAwaitingSubmitEnter はブラケットペースト本文を送った直後に印を立てる。
func (s *Server) setAwaitingSubmitEnter(sessionID int) {
	s.sessionsMu.Lock()
	if ses := s.sessions[sessionID]; ses != nil {
		ses.awaitingSubmitEnter = true
	}
	s.sessionsMu.Unlock()
}

// takeAwaitingSubmitEnter は印を読んで必ず下ろす。本文以外が挟まれば、その入力が
// 印を消費して false になるため、無関係な CR が確定 CR として扱われることはない。
func (s *Server) takeAwaitingSubmitEnter(sessionID int) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil {
		return false
	}
	was := ses.awaitingSubmitEnter
	ses.awaitingSubmitEnter = false
	return was
}

// submitEnterTiming は確定 \r の送出・確認タイミング。ゼロ値は本番既定
// （server.go の submitEnter* 定数）を意味し、テストだけが実時間を待たずに経路を
// 検証するために埋める（relayDeps と同じ「ゼロ値 = 本番」方式）。
type submitEnterTiming struct {
	idleSettle    time.Duration
	minWait       time.Duration
	slowMinWait   time.Duration
	maxWait       time.Duration
	poll          time.Duration
	confirmWindow time.Duration
}

// resolved はゼロのフィールドだけを既定値で埋める（部分上書きを許す）。
func (t submitEnterTiming) resolved() submitEnterTiming {
	if t.idleSettle <= 0 {
		t.idleSettle = submitEnterIdleSettle
	}
	if t.minWait <= 0 {
		t.minWait = submitEnterMinWait
	}
	if t.slowMinWait <= 0 {
		t.slowMinWait = submitEnterSlowMinWait
	}
	if t.maxWait <= 0 {
		t.maxWait = submitEnterMaxWait
	}
	if t.poll <= 0 {
		t.poll = submitEnterPoll
	}
	if t.confirmWindow <= 0 {
		t.confirmWindow = submitEnterConfirmWindow
	}
	return t
}

// minWaitFor は provider 別の最低待機を返す。codex / opencode は大きいペーストを
// プレースホルダへほぼ無出力で畳み込み、静止が瞬時に成立して早撃ちになるため長く取る。
// web/src/app/deferred-enter.ts の deferredEnterMinWaitFor と同じ規準・同じ値。
func (t submitEnterTiming) minWaitFor(provider string) time.Duration {
	switch provider {
	case "codex", "opencode":
		return t.slowMinWait
	default:
		return t.minWait
	}
}

// waitForSubmitEnterSettle は本文を送ってから確定 \r を撃つまで待つ。固定遅延では
// ペースト長・環境速度に依存して当たり外れがあるため、PTY 出力が一定時間静止した
// （= 取り込み・再描画が落ち着いた）ことを待つ。provider 別の最低待機を先に確保し、
// 出力が止まらない病的ケースは maxWait で打ち切って必ず送出する。
// 戻り値は実際に待った時間（テスト・ログ用）。
func (s *Server) waitForSubmitEnterSettle(sessionID int) time.Duration {
	t := s.submitEnter.resolved()
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	var provider string
	if ses != nil {
		provider = ses.Provider
	}
	s.sessionsMu.Unlock()
	if ses == nil {
		return 0
	}
	minWait := t.minWaitFor(provider)
	start := time.Now()
	for {
		elapsed := time.Since(start)
		if elapsed >= t.maxWait {
			s.logger.Warn("submit enter settle timed out; sending anyway",
				"session_id", sessionID, "provider", provider, "waited_ms", elapsed.Milliseconds())
			return elapsed
		}
		if elapsed >= minWait && s.outputQuietFor(sessionID, t.idleSettle) {
			return elapsed
		}
		sleep := t.poll
		if remain := minWait - elapsed; remain > 0 && remain < sleep {
			sleep = remain
		}
		time.Sleep(sleep)
	}
}

// outputQuietFor は直近の PTY 出力から quiet 以上経過しているかを返す。セッションが
// 消えていれば待つ相手がいないので true。lastOutputAt がゼロ（まだ一度も出力していない）
// も、待つべき再描画が存在しないので true とする。
func (s *Server) outputQuietFor(sessionID int, quiet time.Duration) bool {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	var last time.Time
	if ses != nil {
		last = ses.lastOutputAt
	}
	s.sessionsMu.Unlock()
	if ses == nil || last.IsZero() {
		return true
	}
	return time.Since(last) >= quiet
}

// confirmOrResendSubmitEnter は確定 \r の後に CLI が動き出したかを見て、動いていなければ
// 1 回だけ再送する。
//
// 「送れた」は「確定した」ではない。wc.send は ack を持たず、届いた \r を CLI が
// ペースト取り込み中に吸収すると本文が入力欄に載ったまま止まる。Hub 側からは成功と
// 区別できないため警告も出ず、relay が無人で止まっていた（2026-08-29 実測 4 回中 3 回）。
//
// 判定は「\r を書いた後に新しい PTY 出力が来たか」だけで行う。出力が 1 バイトも来て
// いないなら画面は送出前から一切変わっていないので、再送が送出後に現れた別のプロンプトへ
// 当たることは原理的に起きない。再送は 1 回だけにして、二重確定による後続プロンプトの
// 誤承認を避ける（web/src/app/deferred-enter.ts の「\r は必ず 1 回」を Go 側でも守る）。
func (s *Server) confirmOrResendSubmitEnter(sessionID int, wc *wrapperConn, enter string) {
	t := s.submitEnter.resolved()
	if s.waitForOutputAfter(sessionID, time.Now(), t.confirmWindow, t.poll) {
		return
	}
	if s.currentWrapperForInput(sessionID) != wc {
		return
	}
	s.logger.Warn("submit enter had no effect; resending once", "session_id", sessionID)
	if err := s.sendPTYInputFrame(wc, sessionID, enter); err != nil {
		s.logger.Warn("submit enter resend failed", "session_id", sessionID, "err", err)
		return
	}
	if s.waitForOutputAfter(sessionID, time.Now(), t.confirmWindow, t.poll) {
		s.logger.Info("submit enter confirmed after resend", "session_id", sessionID)
		return
	}
	s.logger.Warn("submit enter still not confirmed after resend", "session_id", sessionID)
}

// waitForOutputAfter は since より後の PTY 出力が現れるまで window だけ待つ。
// セッションが消えた場合は待つ意味が無いので true（＝再送しない）で打ち切る。
func (s *Server) waitForOutputAfter(sessionID int, since time.Time, window, poll time.Duration) bool {
	deadline := since.Add(window)
	for {
		s.sessionsMu.Lock()
		ses := s.sessions[sessionID]
		var last time.Time
		if ses != nil {
			last = ses.lastOutputAt
		}
		s.sessionsMu.Unlock()
		if ses == nil {
			return true
		}
		if last.After(since) {
			return true
		}
		if !time.Now().Before(deadline) {
			return false
		}
		time.Sleep(poll)
	}
}

func (s *Server) currentWrapperForInput(sessionID int) *wrapperConn {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if s.sessions[sessionID] == nil {
		return nil
	}
	return s.wrappers[sessionID]
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
	// 保留が積まれたセッションで、この記録が一度も出なければ「吐き出す機会が来ていない」
	// ことの直接の証拠になる（呼び出し元は wrapper の再接続と orchestration の 2 箇所のみ）。
	s.probe("input.flush", "session_id", sessionID,
		"pending", len(pending), "resend", len(resend), "wrapper_connected", wc != nil)
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
		if rem := s.trySendInput(sessionID, combined); rem != "" {
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

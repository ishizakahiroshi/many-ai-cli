package hub

import (
	"sort"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

// 保留中の承認の記録 — このファイルがルールの正本である。
//
// Hub は「このセッションでいま答えを待っている承認」を、session.pendingApproval の
// 1 件の記録として持つ。供給元が端末ミラーのネイティブ検出でも、端末ミラーから読んだ
// マーカーでも、トランスクリプトから読んだマーカーでも、同じ記録に入り、同じ規則で
// 閉じる（docs/local/plan_approval-display-single-source.md）。
//
// なぜ記録にしたか: 以前の Hub は承認の同一性（sig / candidateKey / sourceEpoch）だけを
// ネイティブ用とマーカー用の 2 組で持ち、中身は持っていなかった。画面は Hub から一度きりの
// 通知を受け取るだけで「いまの状態」を渡されず、端末の文字を読んで自分で記録を作り、
// タイマーで取り直していた。その積み重ねが、別のセッションを見ている間に届いた承認が
// 切り替えても描かれない行き止まりを作った（bugfix_approval-panel-blank-on-switch_2026-09-23.md）。
//
// 規則:
//
//   - 1 セッションにつき記録は 1 件。ネイティブ用・マーカー用のように枠を分けない。
//     枠を分けると「いま保留中」が 2 本になり、どちらを描くかを画面が決めることになる。
//   - 新しい候補が来たら、古い記録は閉じる（上書き）。例外は 1 つだけで、端末ミラーから
//     読んだマーカーは、画面に出ているネイティブの記録を上書きしない
//     （approvalRecordBlocksVTMarkerLocked）。ネイティブの承認は CLI を止めて答えを待つので、
//     それが画面に出ている間は、上の scrollback に残っているマーカーが答えを待っていることは
//     ない。端末ミラーのマーカーは出力のたびに読み直されるので、この例外が無いと、古い
//     マーカーとネイティブの承認がチャンクごとに記録を奪い合う。トランスクリプトのマーカーは
//     新しいメッセージでしか来ない一度きりの観測なので、この例外の対象にしない。
//   - 同一性は candidateKey + sourceEpoch だけ（approval_identity.go の 1 本ルール）。
//     記録は「いま保留中の候補」を持つだけで、「同じ質問とは何か」の 2 つ目の定義を持たない。
//     Sig は画面と台帳の行を引き当てる参照で、同一性の判定には使わない
//     （TestApprovalSuppressionStateIsSingleSource が記録型の走査で固定している）。
//   - 閉じ方は approvalClose* の 6 つで、閉じるのは closeApprovalRecordLocked だけ。
//     閉じた記録は台帳（approvals）の行を resolved にする。history reset だけは台帳の行ごと
//     消える（sessionstore.ClearSessionHistory）ので更新しない。
//   - 記録は値として扱い、開いた後に書き換えない。変えるときは新しい記録で置き換える。
//     reattach で旧セッションから新セッションへポインタを渡しても共有状態にならない。
//   - 台帳への書き込みと配信は sessionsMu を外してから行う（finishApprovalRecordClosures）。
//     古い記録の resolved は、新しい記録の INSERT より先に書く。同じ sig の候補が世代を
//     変えて開き直したとき、逆順だと新しい行まで resolved にしてしまう。

// Origin は「どう答え、どう閉じるか」で 2 つに分ける。
//
//   - native: CLI 自身の承認画面と、Claude の AskUserQuestion の告知
//     （approval_detector.go の approvalKindAskUserQuestion）。端末でキーを押して答え、
//     画面から消えたら閉じる（approvalCloseVanished）。
//   - marker: AI が文章で書いた質問。承認マーカーと、マーカー無しの文章の質問
//     （approval_text_question.go の Yes/No・順次質問・旧形式の選択）。次のユーザーの発話が
//     答えで、確定したユーザーターンかトランスクリプトの user メッセージで閉じる。
//     名前は歴史的経緯で marker のまま（以前は承認マーカーしか無かった）。
//
// 画面へ知らせるのは approval_state（開く・閉じる）と、接続した画面へのまとめ
// （approval_snapshot）だけ。画面はこの記録を描くだけで、端末の文字から承認を作らない
// （web/src/app/approval-store.ts）。以前の承認ごとの通知（approval_detected / approval_marker /
// approval_cleared）は送らない（TestApprovalLegacyMessagesAreNotSent が固定している。画面側は
// scripts/check-approval-display-source.mjs）。
const (
	approvalRecordOriginNative = "native"
	approvalRecordOriginMarker = "marker"
)

// 記録を閉じた理由。画面へ知らせる閉じるメッセージにもそのまま載る。
const (
	// 画面の回答（approval_consumed）・one-tap・自動承認・一括承認。Hub を経由した回答。
	approvalCloseAnswered = "answered"
	// 端末での回答。確定したユーザーターン（ブラウザの端末・入力欄・パネルの選択肢送信・
	// Hub からの送信）と、トランスクリプトに現れた user メッセージ。
	approvalCloseAnsweredTerminal = "answered_terminal"
	// 新しい候補に置き換わった。
	approvalCloseSuperseded = "superseded"
	// ネイティブの承認が画面から消えた（nativeApprovalClearMissLimit 回続けて検出されない）。
	// トランスクリプトの文章の質問の後に AI が答えを待たずに話し続けたときも、これで閉じる
	// （approval_text_question.go の「出力が落ち着いてから開く」節）。
	approvalCloseVanished = "vanished"
	// セッションが終わった（wrapper の切断・dismiss）。
	approvalCloseSessionEnd = "session_end"
	// 利用者が history reset を実行した。
	approvalCloseHistoryReset = "history_reset"
)

// approvalRecord は保留中の承認 1 件。開いた後は書き換えない（ファイル冒頭の規則）。
type approvalRecord struct {
	// 同一性（approval_identity.go の 1 本）。
	CandidateKey   string
	CandidateShape string
	SourceEpoch    uint64
	// Sig は画面・台帳から記録を引き当てる参照。ネイティブは選択肢と文脈の署名、
	// マーカーはブロック本文の署名で、どちらも台帳の行のキーと同じ値。同一性の判定には使わない。
	Sig string

	// Origin は native / marker、Source は go_vt / transcript。
	Origin string
	Source string
	// Kind は承認の種類（marker / native / native_codex_shortcut / ask_user_question ...）。
	Kind string

	// 本文。承認マーカーはブロック原文だけを持ち、解釈は画面側のパーサ 1 本に任せる
	// （approval_marker.go の方針を変えない）。ネイティブは Go 側で解いた質問と選択肢を持つ。
	// マーカー無しの文章の質問は、画面のパーサが読める形に整えた Block と、解いた質問・選択肢の
	// 両方を持つ（approval_text_question.go）。AskUserQuestion の告知は質問文だけを持つ。
	Block    string
	Question string
	Context  string
	Options  []proto.ApprovalOption
	Summary  proto.ApprovalSummary

	DetectedAt time.Time
}

func (r *approvalRecord) isNative() bool {
	return r != nil && r.Origin == approvalRecordOriginNative
}

func (r *approvalRecord) isMarker() bool {
	return r != nil && r.Origin == approvalRecordOriginMarker
}

// hasOptions は Web のボタンから答えられる記録か（告知と承認マーカーは選択肢を持たない）。
// 自動承認・一括承認・ワンタップは選択肢のある記録にだけ効く。
func (r *approvalRecord) hasOptions() bool {
	return r != nil && len(r.Options) > 0
}

// isCandidate は記録が candidateKey + sourceEpoch の候補そのものかを返す。
func (r *approvalRecord) isCandidate(candidateKey string, sourceEpoch uint64) bool {
	return r != nil && candidateKey != "" && r.CandidateKey == candidateKey && r.SourceEpoch == sourceEpoch
}

// wire は記録を画面へ送る形にする。マーカーは原文だけ、ネイティブは解いた本文を送る。
func (r *approvalRecord) wire() *proto.ApprovalRecord {
	if r == nil {
		return nil
	}
	out := &proto.ApprovalRecord{
		CandidateKey:   r.CandidateKey,
		CandidateShape: r.CandidateShape,
		SourceEpoch:    r.SourceEpoch,
		Sig:            r.Sig,
		Origin:         r.Origin,
		Source:         r.Source,
		Kind:           r.Kind,
		Block:          r.Block,
		Question:       r.Question,
		Context:        r.Context,
		Options:        append([]proto.ApprovalOption(nil), r.Options...),
	}
	if r.isNative() {
		summary := r.Summary
		out.Summary = &summary
	}
	if !r.DetectedAt.IsZero() {
		out.DetectedAt = r.DetectedAt.Format(time.RFC3339)
	}
	return out
}

// openApprovalRecordLocked は記録を開き、画面へ送る approval_state（開く）と、
// 「保留中」が変わったときの session_update（変わらなければ nil）を返す。呼び出し側は
// sessionsMu を持ち、返り値の配信は解放後に行う。古い記録があれば、先に
// closeApprovalRecordLocked で閉じておく（版番号は閉じる・開くの順に 1 ずつ進む）。
// そのとき閉じた側の session_update は捨てる（開いた側が最終の状態を持つ。両方送ると
// 「保留中」が一瞬下りて見える）。
//
// 版番号は同一性ではなく、1 セッションの中の並び順だけに使う。配信はロックを外してから
// 行うので、開く・閉じるが逆順に届くことがある。画面は手元より古い版を捨てるので、
// 閉じた記録が遅れて届いた「開く」で生き返らない。
func openApprovalRecordLocked(ses *session, id int, record *approvalRecord) (proto.Message, *proto.Message) {
	ses.pendingApproval = record
	ses.approvalStateVersion++
	state := proto.Message{
		Type:          "approval_state",
		SessionID:     id,
		Provider:      ses.Provider,
		ApprovalState: &proto.ApprovalState{Version: ses.approvalStateVersion, Open: record.wire()},
	}
	return state, refreshApprovalActivityLocked(ses)
}

// refreshApprovalActivityLocked は「保留中」（AwaitingApproval / AwaitingUser）を記録の
// 有無から計算し直し、表示が変わったら送る session_update を返す。
//
// 「保留中」はこの記録からだけ出す（docs/local/plan_approval-display-single-source.md の
// 原則 2）。以前は画面の申告（session_hint。撤去済み）だけから立ち、画面を 1 つも開いて
// いないとマーカーの承認で立たなかった。
// 終わったセッションは表示を動かさない（フラグだけ下ろし、配信しない）。
func refreshApprovalActivityLocked(ses *session) *proto.Message {
	before, beforeState := ses.Activity, ses.State
	setAwaitingFromApprovalRecordLocked(ses)
	ses.Activity.Normalize()
	if isTerminalSessionState(ses.State) {
		return nil
	}
	ses.State = ses.Activity.DisplayState()
	if before == ses.Activity && beforeState == ses.State {
		return nil
	}
	update := sessionUpdateMessage(ses)
	return &update
}

// setAwaitingFromApprovalRecordLocked は「保留中」（AwaitingApproval / AwaitingUser）を記録の
// 有無だけから立て、立てたかを返す。AwaitingApproval へ書くのはこの関数だけ
// （TestAwaitingApprovalIsAssignedOnlyByApprovalRecord がソースの走査で固定している）。
// 出力の受信（markRunning）と idle の判定（evaluateIdle）もここを通すので、画面の申告や
// 端末の文字で「保留中」が立つ経路は無い。呼び出し側は sessionsMu を持つ。
func setAwaitingFromApprovalRecordLocked(ses *session) bool {
	awaiting := ses.pendingApproval != nil
	ses.Activity.AwaitingApproval = awaiting
	ses.Activity.AwaitingUser = awaiting
	return awaiting
}

// approvalNotificationIDLocked は承認の通知（Web Push・ntfy / webhook）の重複抑止に使う ID を返す。
//
// 通知は記録が開いたときに 1 回だけ出す。送信側も ID で 1 時間の重複抑止をするので、sig を
// そのまま ID にすると、ユーザーのターンをはさんで同じ質問がもう一度来たとき（別の記録）に
// 抑止に当たって鳴らない。世代を足して記録ごとの ID にする。同じ世代で開き直した記録
// （ネイティブの承認が一時的に見えなくなって開き直したときなど）は同じ ID になり、鳴り直さない。
// ワンタップの token は sig で発行する（呼び出し側）。この ID は重複抑止にだけ使う。
func approvalNotificationIDLocked(ses *session, sig string) string {
	if r := ses.pendingApproval; r != nil && sig != "" && r.Sig == sig {
		return sig + "#" + strconv.FormatUint(r.SourceEpoch, 10)
	}
	return sig
}

// approvalSnapshotMessage は全セッションの記録と版番号を 1 通にまとめる。接続した画面に
// だけ送る（sendSnapshot）。記録の無いセッションも Record=nil で並べるので、画面は手元の
// 記録を丸ごと置き換えられ、前の接続で持っていた古い記録が残らない。
func (s *Server) approvalSnapshotMessage() proto.Message {
	s.sessionsMu.Lock()
	states := make([]proto.ApprovalSessionState, 0, len(s.sessions))
	for id, ses := range s.sessions {
		if ses == nil || ses.UsageProbe {
			continue
		}
		states = append(states, proto.ApprovalSessionState{
			SessionID: id,
			Version:   ses.approvalStateVersion,
			Record:    ses.pendingApproval.wire(),
		})
	}
	s.sessionsMu.Unlock()
	sort.Slice(states, func(i, j int) bool { return states[i].SessionID < states[j].SessionID })
	return proto.Message{Type: "approval_snapshot", ApprovalSnapshot: states}
}

// handleApprovalResync は画面からの問い直し（approval_resync。抑止告知の「再検出」）に答える。
//
//  1. そのセッションの端末ミラーから、今の候補を評価し直す（evaluateReplayApproval）。
//     同じ候補を二度配信しない抑止はそのまま効く。新しく記録ができた・変わったときは状態の
//     変化なので、いつもどおり全画面へ approval_state を送る（原則 3）。
//     トランスクリプトが供給元のセッションは、次の poll が読み直すのでここでは読まない。
//  2. そのセッションの今の記録と版番号を、問い直した画面にだけ送る。ほかの画面は状態が
//     変わっていないので送らない。画面は手元以下の版を捨てるので、既に持っている画面では
//     何も変わらず、取りこぼした画面だけがここで今の記録に追いつく。記録が無いときは
//     Open も Close も空の approval_state（その版では記録が無い）を送る。
//
// 以前の画面は「↻ 承認」と再試行で自分の写しから描き直していたが、写しは Hub の記録から
// しか作られなくなったので、取り直す先は Hub になった。
func (s *Server) handleApprovalResync(uc *uiConn, id int) {
	if uc == nil || id <= 0 {
		return
	}
	s.evaluateReplayApproval(id)
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.UsageProbe {
		s.sessionsMu.Unlock()
		return
	}
	msg := proto.Message{
		Type:          "approval_state",
		SessionID:     id,
		Provider:      ses.Provider,
		ApprovalState: &proto.ApprovalState{Version: ses.approvalStateVersion, Open: ses.pendingApproval.wire()},
	}
	// 接続直後のまとめを送っている間は、broadcast と同じく後ろに積む（まとめを追い越さない）。
	if uc.priming || uc.draining {
		uc.queued = append(uc.queued, msg)
		s.sessionsMu.Unlock()
		return
	}
	s.sessionsMu.Unlock()
	if err := uc.sendWithDeadline(msg, time.Now().Add(broadcastWriteTimeout)); err != nil {
		s.logger.Warn("approval_resync: UI send failed, removing dead connection", "err", err)
		s.removeUI(uc.ws)
	}
}

// approvalRecordBlocksVTMarkerLocked は、端末ミラーから読んだマーカーで記録を上書きしては
// いけない状態かを返す（ファイル冒頭の規則の例外）。直近の走査でネイティブの承認がまだ
// 画面に見えている（取りこぼしが 0 回）間だけ守る。1 回でも見えなくなったら、消えかけの
// ネイティブより画面に出ているマーカーを優先する。呼び出し側は sessionsMu を持つ。
func approvalRecordBlocksVTMarkerLocked(ses *session) bool {
	return ses != nil && ses.pendingApproval.isNative() && ses.nativeApprovalClearMisses == 0
}

// approvalRecordClosure は閉じた記録と、sessionsMu を外した後にやることを持つ。
type approvalRecordClosure struct {
	sessionID int
	record    *approvalRecord
	reason    string
	closedAt  time.Time
	// answer は台帳の selected_text に残す文字列（回答の入力）。回答以外の閉じ方では空。
	answer string
	// state は画面へ送る approval_state（閉じる）。
	state proto.Message
	// activity は「保留中」が変わったときの session_update（変わらなければ nil）。
	activity *proto.Message
}

// closeApprovalRecordLocked はセッションの記録を閉じる。記録が無ければ nil を返す。
// 呼び出し側は sessionsMu を持ち、返り値を解放後に finishApprovalRecordClosures へ渡す。
func closeApprovalRecordLocked(ses *session, id int, reason string, now time.Time) *approvalRecordClosure {
	if ses == nil || ses.pendingApproval == nil {
		return nil
	}
	record := ses.pendingApproval
	ses.pendingApproval = nil
	ses.nativeApprovalClearMisses = 0
	ses.approvalStateVersion++
	return &approvalRecordClosure{
		sessionID: id, record: record, reason: reason, closedAt: now,
		activity: refreshApprovalActivityLocked(ses),
		state: proto.Message{
			Type:      "approval_state",
			SessionID: id,
			Provider:  ses.Provider,
			ApprovalState: &proto.ApprovalState{
				Version: ses.approvalStateVersion,
				Close: &proto.ApprovalRecordClose{
					CandidateKey: record.CandidateKey,
					SourceEpoch:  record.SourceEpoch,
					Sig:          record.Sig,
					Origin:       record.Origin,
					Reason:       reason,
				},
			},
		},
	}
}

// finishApprovalRecordClosures は閉じた記録の台帳を resolved にし、画面へ閉じたことを
// 知らせる（approval_state）。sessionsMu を持たずに呼ぶ。
func (s *Server) finishApprovalRecordClosures(closures ...*approvalRecordClosure) {
	for _, c := range closures {
		if c == nil || c.record == nil {
			continue
		}
		if s.sessionStore != nil && c.record.Sig != "" && c.reason != approvalCloseHistoryReset {
			s.sessionStore.StoreApprovalConsumed(c.sessionID, c.record.Sig, c.answer, c.closedAt)
		}
		s.broadcast(c.state)
		if c.activity != nil {
			s.broadcast(*c.activity)
		}
	}
}

// dropActivity は、閉じた直後に同じ区間で新しい記録を開く（上書き）・セッションを
// 消す（dismiss）ときに、閉じた側の session_update を捨てる。
func (c *approvalRecordClosure) dropActivity() {
	if c != nil {
		c.activity = nil
	}
}

// transcriptAnswerTextLimit は台帳の selected_text に残す回答の上限。一覧表示用なので
// 先頭だけあればよい。
const transcriptAnswerTextLimit = 200

func truncateApprovalAnswer(answer string) string {
	answer = strings.TrimSpace(answer)
	if runes := []rune(answer); len(runes) > transcriptAnswerTextLimit {
		answer = string(runes[:transcriptAnswerTextLimit])
	}
	return answer
}

// confirmedTurnText は送られた入力が確定したユーザーターンなら、その本文を返す。
// チャット本文はブラケットペースト包み（... \x1b[201~）と確定 \r の別送で届くため、
// 末尾の \r だけでなくペースト終端も確定として扱う。後続の確定 \r は剥がすと空文字になり、
// 二重に数えない。
func confirmedTurnText(raw string) (string, bool) {
	if !strings.HasSuffix(raw, "\r") && !strings.HasSuffix(raw, bracketedPasteEnd) {
		return "", false
	}
	text := strings.TrimRight(raw, "\r\n")
	text = strings.ReplaceAll(text, bracketedPasteStart, "")
	text = strings.ReplaceAll(text, bracketedPasteEnd, "")
	return text, text != ""
}

// closeMarkerRecordOnSubmittedTurn は、確定したユーザーターンが CLI へ送られたとき、
// 保留中のマーカーの記録を閉じる。マーカーの質問への答えは次のユーザーターンそのもので、
// 質問に答えずに別の指示を送った場合も、その質問はもう答えを待っていない。
//
// 端末ミラーのマーカーは、これまで Hub に閉じる判定が無く、画面が端末の文字と照合して
// 閉じていた。画面の文字照合は Ink の差分再描画で本文が欠けると未回答の承認まで閉じるので、
// Hub へは持ち込まず、ユーザーターンを正にする。トランスクリプトのマーカーも同じ合図で閉じる
// （トランスクリプトの user メッセージは、その後に届いて何もしない）。
//
// ネイティブの承認はキー 1 つで答えるので、ここでは閉じない（approval_consumed・消失で閉じる）。
//
// 閉じたことは approval_state で全部の画面へ届くので、別の画面や端末へ直接答えた画面の
// パネルもこれで閉じる。後から届く approval_consumed とトランスクリプトの user メッセージは、
// 記録が無いので何もしない。
//
// 世代は進めない。回答の Enter は画面からの approval_consumed より先に届くので、ここで
// 世代を進めると、遅れて届いた approval_consumed が 1 つ前の世代を回答済みに書き戻し、
// 持ち越しの判定（markApprovalUserTurnBoundaryLocked）を壊す。回答済みは記録の世代で残し、
// 後から来た approval_consumed は同じ状態を書くだけにする（冪等）。
func (s *Server) closeMarkerRecordOnSubmittedTurn(id int, text string, now time.Time) bool {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses != nil {
		// まだ開いていない文章の質問の候補も、このターンで答えを待たなくなった。
		ses.textQuestionAtIdle = nil
	}
	if ses == nil || !ses.pendingApproval.isMarker() {
		s.sessionsMu.Unlock()
		return false
	}
	record := ses.pendingApproval
	markApprovalConsumedAtEpochLocked(ses, record.CandidateKey, record.Sig, record.SourceEpoch)
	closure := closeApprovalRecordLocked(ses, id, approvalCloseAnsweredTerminal, now)
	closure.answer = truncateApprovalAnswer(text)
	provider := ses.Provider
	s.sessionsMu.Unlock()

	s.finishApprovalRecordClosures(closure)
	if s.logger != nil {
		s.logger.Info("approval marker closed by submitted user turn",
			"session_id", id, "provider", provider, "source", record.Source, "sig", shortSig(record.Sig))
	}
	return true
}

package hub

import (
	"bytes"
	"strings"
	"time"
)

// approvalSourceTranscript は「CLI 自身のトランスクリプトから読んだ承認マーカー」を表す。
// go_vt（端末ミラー）と区別できるようにするためのラベルで、Web 側の表示分岐には
// 使わない（ブラウザは approval_marker を受け取った時点で hub_marker として扱う）。
const approvalSourceTranscript = "transcript"

// approvalMarkerTranscriptMissLimit はトランスクリプトを読めない poll が何回続いたら
// 供給元を VT ミラーへ戻すか。理由は下の「読めなくなったとき」節。
const approvalMarkerTranscriptMissLimit = 3

// 承認マーカーの供給元 — claude / codex は端末ミラーではなくトランスクリプトを読む。
//
// なぜ移したか（2026-08-29 実測・docs/local/bugfix_approval-marker-block-overflows-screen_2026-08-29.md）:
// 承認マーカーブロックは AI から Hub への連絡だが、それを「端末の表示」という
// 容量が窓の高さで決まる経路で運んでいた。Claude Code は代替画面を Ink が絶対座標で
// 塗り直すので、画面高に収まらない回答はあふれた行がスクロールアウトではなく単に
// 描かれない。実測では開始マーカーが端末へ 1 バイトも届かず（vtBuffer の scrollback は
// newLine() で押し出された行しか積まないのでそこにも入らない）、Hub がどこを読んでも
// 取り出せない状態になっていた。同じ回答は CLI 自身のトランスクリプトに、端末の
// 大きさと無関係な完全な形で、端末が終了マーカーを描いたのと同じ秒に入っている。
//
// 「終了マーカーが下端から何行以内なら本文とみなすか」を緩める案は採っていない。
// 閾値が動くだけで、後書きがもう少し長い回答が来れば同じ場所で落ちるため。
//
// 供給元は 1 セッションにつき 1 つに保つ。VT とトランスクリプトの両方から同じ質問を
// 立てると、端末の折り返しで質問文が切れている側と切れていない側で candidateKey が
// 割れ、承認の同一性（approval_identity.go の 1 本ルール）が実質 2 本になる。だから
// この関数が true のセッションでは VT 側の抽出を呼ばない（wrapper_loop.go の pty_data と
// approval_native.go の evaluateReplayApproval）。
//
// トランスクリプトを読めない provider（copilot / cursor-agent / opencode / grok）と、
// パスがまだ 1 度も解決できていない間は、これまでどおり VT ミラーが供給元になる。
// agentChatPath はポーラーがファイルを実際に見つけた後にだけ埋まるので、
// 「読める見込み」ではなく「読めた」ことを条件にできる。
//
// 読めなくなったとき（敵対レビュー Finding 2 / 2026-08-29）:
//
// 供給元をトランスクリプトへ固定したまま戻れないと、ファイルが移動・削除された、
// パースが通らない、といった理由で読めなくなった瞬間に承認が無音で沈黙する。
// 読み取り失敗は Debug ログ 1 行にしかならないので、利用者からも開発者からも
// 見えない。**それはこの bugfix が消そうとした失敗の型そのもの**なので、
// 連続 approvalMarkerTranscriptMissLimit 回の poll でトランスクリプトを読めなければ
// VT ミラーへ退避し、読めるようになったら戻る。
//
// 回数で切るのは、poll が 1 秒周期の固定リズムを持っていて「何秒読めていないか」を
// 別に測らなくても回数がそのまま経過時間になるから。3 回にしたのは、ログの
// ローテーションや `--resume` でのファイル差し替えのような一過性の失敗で供給元を
// 揺らさない一方、承認を待たせる長さとしては 3 秒が上限として妥当なため。
// 退避のときだけ Warn を 1 本出す（無音で切り替えない）。
//
// 退避で受け入れる代償: 承認が出たまま供給元が入れ替わると、同じ質問が VT 側の
// candidateKey でもう一度出ることがある。承認が二重に見えるのは、承認が出ないより
// はるかに軽い。
func approvalMarkerSourceIsTranscriptLocked(ses *session) bool {
	return ses != nil && isAgentChatProvider(ses.Provider) && ses.agentChatPath != "" &&
		ses.agentChatMissStreak < approvalMarkerTranscriptMissLimit
}

// approvalMarkerFromTranscriptText は assistant メッセージ 1 通の本文から承認マーカー
// ブロックを取り出す。本文は端末の折り返しも塗り直しも通っていないので、VT 側にある
// 開始マーカー欠落時の再構成（reconstructOpenlessMarkerBlock）は要らない。
func approvalMarkerFromTranscriptText(text string) *approvalMarkerBlock {
	if !strings.Contains(text, approvalMarkerClose) {
		return nil
	}
	return extractApprovalMarkerBlock(strings.Split(text, "\n"))
}

// scanTranscriptApprovalMarkers はトランスクリプトのポーリングで新しく読めた
// メッセージから承認マーカーを配信する。
//
// prime（reattach 等でパーサ状態を作り直したときの 1 回目）は、すでに書かれている
// レコードからブラウザの表示を組み直すだけの baseline であって新しい観測ではない
// （codex の task_complete を prime で publish しないのと同じ理由）。ただし最後の
// メッセージだけは例外にする。それが assistant のマーカーなら「まだ誰も答えていない
// 質問がそこで止まっている」状態そのものなので、reattach 後に承認パネルが戻らない。
// 回答済みだった場合は candidateKey + sourceEpoch がそのまま抑止するので、
// ここに 2 本目の抑止を足す必要はない。
// 供給元の判定に session の agentChatPath を読み直さないこと。それを書くのは
// pollAgentChat の末尾で、走査はその手前にある。session 側を見ると「これから書く値」を
// 先に読むことになり、**パスを初めて解決した poll のバッチが丸ごと捨てられる**
// （Hub 再起動後の初回登録・--resume でパスが変わったとき・セッションの初回登録）。
// 捨てられるのは prime のバッチ、つまり下の「最後のメッセージだけは復元する」が
// いちばん効いてほしい場面そのものだった。poll がその場で持っている値を渡す。
//
// 質問を立てるのと同じ経路で、質問を下ろす（2026-09-08 追記・
// bugfix_approval-panel-lost-after-transcript-marker_2026-09-08.md）:
// トランスクリプトに次の user メッセージが現れたら、その前に立っていたマーカー承認は
// 答えられている（または捨てられている）。ブラウザ経由の回答なら approval_consumed で
// 先に下りているので何も起きない。端末へ直接答えたときはこの経路だけが閉じる合図になる。
// 供給元をトランスクリプトへ移した以上、「答えたかどうか」も端末の画面ではなく
// トランスクリプトで判定する。画面の文字照合で閉じる旧経路（ブラウザ側の H9 / bg）は、
// Ink の差分再描画で本文が欠けると未回答の承認まで閉じてしまい、閉じた承認を
// 再配信する経路が無いので、この供給元では使わない。
func (s *Server) scanTranscriptApprovalMarkers(id int, provider, transcriptPath string, messages []agentChatMessage, prime bool, detectedAt time.Time) {
	if len(messages) == 0 || transcriptPath == "" || !isAgentChatProvider(provider) {
		return
	}
	if prime {
		last := messages[len(messages)-1]
		switch last.Role {
		case "assistant":
			if marker := approvalMarkerFromTranscriptText(last.Text); marker != nil {
				s.maybeBroadcastApprovalMarkerFrom(id, marker, detectedAt, approvalSourceTranscript)
			}
		case "user":
			// 最後が user なら、それより前に立っていた質問はもう止まっていない。
			s.closeApprovalMarkerOnTranscriptUserMessage(id, last.Text, detectedAt)
		}
		return
	}
	for _, message := range messages {
		if message.Text == "" {
			continue
		}
		switch message.Role {
		case "user":
			s.closeApprovalMarkerOnTranscriptUserMessage(id, message.Text, detectedAt)
		case "assistant":
			if marker := approvalMarkerFromTranscriptText(message.Text); marker != nil {
				s.maybeBroadcastApprovalMarkerFrom(id, marker, detectedAt, approvalSourceTranscript)
			}
		}
	}
}

// transcriptAnswerTextLimit は台帳の selected_text に残す user メッセージの上限。
// 一覧表示用なので先頭だけあればよい。
const transcriptAnswerTextLimit = 200

// closeApprovalMarkerOnTranscriptUserMessage は、トランスクリプトに user メッセージが
// 現れた時点で保留中のマーカー承認を回答済みにする。ブラウザからの approval_consumed
// （markNativeApprovalConsumed）と同じ状態遷移を Hub 自身の判断で起こすので、
// 遅延フレーム向けの世代ずれ判定は通さない。保留中の候補が無ければ何もしない。
func (s *Server) closeApprovalMarkerOnTranscriptUserMessage(id int, answer string, now time.Time) bool {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.approvalMarkerCandidateKey == "" {
		s.sessionsMu.Unlock()
		return false
	}
	sig := ses.approvalMarkerSig
	markApprovalConsumedAtEpochLocked(ses, ses.approvalMarkerCandidateKey, sig, ses.approvalMarkerSourceEpoch)
	cleared := clearApprovalMarkerCandidateLocked(ses, id, approvalSourceTranscript)
	provider := ses.Provider
	s.sessionsMu.Unlock()

	answer = strings.TrimSpace(answer)
	if runes := []rune(answer); len(runes) > transcriptAnswerTextLimit {
		answer = string(runes[:transcriptAnswerTextLimit])
	}
	if s.sessionStore != nil && sig != "" {
		s.sessionStore.StoreApprovalConsumed(id, sig, answer, now)
	}
	if s.logger != nil {
		s.logger.Info("approval marker closed by transcript user message",
			"session_id", id, "provider", provider, "sig", shortSig(sig))
	}
	s.broadcast(cleared)
	return true
}

// approvalMarkerCloseBytes は PTY チャンクを走査するための終了マーカー。
var approvalMarkerCloseBytes = []byte(approvalMarkerClose)

// ptyChunkClosesApprovalMarker は「そのチャンクで終了マーカーが端末に出たか」を返す。
// 出た時点で AI は回答を書き終えているので、トランスクリプトの poll を 1 回だけ
// 前倒しする合図に使う（kickAgentChatPollLocked）。
//
// チャンク境界やチロームの塗り直しで取り逃すことはあるが、それでよい。前倒しは
// 体感を保つためのもので、取りこぼしは 1 秒周期の通常 poll が必ず拾う。
func ptyChunkClosesApprovalMarker(data []byte) bool {
	return bytes.Contains(data, approvalMarkerCloseBytes)
}

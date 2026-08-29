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
func approvalMarkerSourceIsTranscriptLocked(ses *session) bool {
	return ses != nil && isAgentChatProvider(ses.Provider) && ses.agentChatPath != ""
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
func (s *Server) scanTranscriptApprovalMarkers(id int, messages []agentChatMessage, prime bool, detectedAt time.Time) {
	if len(messages) == 0 {
		return
	}
	s.sessionsMu.Lock()
	transcriptSource := approvalMarkerSourceIsTranscriptLocked(s.sessions[id])
	s.sessionsMu.Unlock()
	if !transcriptSource {
		return
	}
	if prime {
		last := messages[len(messages)-1]
		if last.Role != "assistant" {
			return
		}
		if marker := approvalMarkerFromTranscriptText(last.Text); marker != nil {
			s.maybeBroadcastApprovalMarkerFrom(id, marker, detectedAt, approvalSourceTranscript)
		}
		return
	}
	for _, message := range messages {
		if message.Role != "assistant" || message.Text == "" {
			continue
		}
		if marker := approvalMarkerFromTranscriptText(message.Text); marker != nil {
			s.maybeBroadcastApprovalMarkerFrom(id, marker, detectedAt, approvalSourceTranscript)
		}
	}
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

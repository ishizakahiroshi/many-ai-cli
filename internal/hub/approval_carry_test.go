package hub

import (
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// 回答済みの端末ミラーの質問が、ネイティブの承認や Hub の再起動をはさんでも開き直らないこと
// （docs/local/bugfix_approval-answered-vt-question-resurrects_2026-09-23.md）。
//
// 端末ミラーの質問は出力のたびに（文章の質問は出力が落ち着くたびに）読み直されるので、
// 答え終わった質問も scrollback に残っている間は毎回取り出される。それを新しい質問と
// 取り違えないのは、回答済みの持ち越し（approval_identity.go の carryConsumedVTQuestionLocked）と、
// メモリが知らないときに台帳から戻す経路（approval_text_question.go の vtQuestionAnsweredInLedger）。

var migrationMarkerLines = []string{
	approvalMarkerOpen,
	"Q1 Proceed with the migration?",
	" 1. Yes (Recommended)",
	" 2. No",
	" N. User specifies",
	approvalMarkerClose,
}

// answeredVTMarkerSession は、端末ミラーのマーカーを開いて入力欄から答えた後の grok セッションを作る。
// withLedger なら承認台帳を持つ（台帳から戻す経路を通す）。
func answeredVTMarkerSession(t *testing.T, withLedger bool) (*Server, *session) {
	t.Helper()
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	if withLedger {
		newApprovalLedger(t, s, 1, "grok")
	}
	ses.vt = newVTBuffer(80, 8)
	ses.vt.scrollback = append([]string(nil), migrationMarkerLines...)
	if !s.maybeBroadcastApprovalMarker(1, extractApprovalMarkerBlockFromVT(ses.vt), time.Now()) {
		t.Fatal("端末ミラーのマーカーが開かない")
	}
	s.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
	if ses.pendingApproval != nil {
		t.Fatalf("入力欄から答えても記録が閉じない: %+v", ses.pendingApproval)
	}
	if reopenVTMarker(s, ses) {
		t.Fatal("前提: 答えた直後に同じマーカーが開き直った")
	}
	return s, ses
}

// reopenVTMarker は次の PTY チャンクと同じく、端末ミラーから最新のマーカーを取り出して開こうとする。
func reopenVTMarker(s *Server, ses *session) bool {
	return s.maybeBroadcastApprovalMarker(ses.ID, extractApprovalMarkerBlockFromVT(ses.vt), time.Now())
}

func TestAnsweredVTMarkerStaysAnsweredAcrossNativeApproval(t *testing.T) {
	// 台帳が無くてもメモリの持ち越しだけで直る経路（案 B）。
	t.Run("ネイティブの承認が 1 回消えた", func(t *testing.T) {
		s, ses := answeredVTMarkerSession(t, false)
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		if !ses.pendingApproval.isNative() {
			t.Fatalf("前提: ネイティブの承認が開かない: %+v", ses.pendingApproval)
		}
		s.handleNativeApprovalDetection(1, nil)
		if reopenVTMarker(s, ses) {
			t.Fatalf("回答済みのマーカーが開き直った: %+v", ses.pendingApproval)
		}
	})

	// ネイティブの回答済みが枠を上書きするので、台帳から戻す経路が要る。
	t.Run("ネイティブの承認に画面から答えた", func(t *testing.T) {
		s, ses := answeredVTMarkerSession(t, true)
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		native := ses.pendingApproval
		s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: native.Sig, ApprovalCandidateKey: native.CandidateKey, ApprovalSourceEpoch: native.SourceEpoch})
		if ses.pendingApproval != nil {
			t.Fatalf("前提: ネイティブの記録が閉じない: %+v", ses.pendingApproval)
		}
		if reopenVTMarker(s, ses) {
			t.Fatalf("回答済みのマーカーが開き直った: %+v", ses.pendingApproval)
		}
		// 答えたネイティブの承認は、画面に残っている間は開き直らない（枠を書き換えていない）。
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		if ses.pendingApproval != nil {
			t.Fatalf("答えたネイティブの承認が開き直った: %+v", ses.pendingApproval)
		}
		if reopenVTMarker(s, ses) {
			t.Fatalf("ネイティブの承認が残っている間に回答済みのマーカーが開き直った: %+v", ses.pendingApproval)
		}

		// 次のユーザーターンで枠が空き、持ち越しがメモリへ戻る。以後は台帳を読まない。
		s.handleInput(proto.Message{SessionID: 1, Text: "続けてください\r"})
		if reopenVTMarker(s, ses) {
			t.Fatalf("次のターンで回答済みのマーカーが開き直った: %+v", ses.pendingApproval)
		}
		s.sessionsMu.Lock()
		consumed, pending := ses.approvalConsumedCandidateKey, ses.approvalEpochPending
		s.sessionsMu.Unlock()
		if consumed != approvalMarkerCandidateIdentity("grok", extractApprovalMarkerBlockFromVT(ses.vt).Block).key || !pending {
			t.Fatalf("持ち越しがメモリへ戻っていない: consumed=%q pending=%v", consumed, pending)
		}
	})
}

// Hub を再起動すると回答済みはメモリごと消える。wrapper の replay から組み直した端末ミラーに
// 残る答えた質問は、台帳から回答済みを戻して開かない。答えていなかった質問は開く。
func TestAnsweredVTMarkerStaysAnsweredAfterHubRestart(t *testing.T) {
	restart := func(t *testing.T, answer bool) (*Server, *session) {
		t.Helper()
		before := newTestServer()
		ses := registerTestSession(before, 1, "grok")
		store := newApprovalLedger(t, before, 1, "grok")
		ses.vt = newVTBuffer(80, 8)
		ses.vt.scrollback = append([]string(nil), migrationMarkerLines...)
		if !reopenVTMarker(before, ses) {
			t.Fatal("前提: 端末ミラーのマーカーが開かない")
		}
		if answer {
			before.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
		}

		after := newTestServer()
		after.sessionStore = store
		restarted := registerTestSession(after, 1, "grok")
		restarted.vt = newVTBuffer(80, 8)
		restarted.vt.scrollback = append([]string(nil), migrationMarkerLines...)
		after.evaluateReplayApproval(1)
		return after, restarted
	}

	t.Run("答えた質問は開かない", func(t *testing.T) {
		s, ses := restart(t, true)
		if ses.pendingApproval != nil {
			t.Fatalf("再起動の後に回答済みのマーカーが開いた: %+v", ses.pendingApproval)
		}
		if reopenVTMarker(s, ses) {
			t.Fatalf("再起動の後の次のチャンクで回答済みのマーカーが開いた: %+v", ses.pendingApproval)
		}
	})
	t.Run("答えていなかった質問は戻る", func(t *testing.T) {
		_, ses := restart(t, false)
		if !ses.pendingApproval.isMarker() {
			t.Fatalf("再起動の後に未回答のマーカーが戻らない: %+v", ses.pendingApproval)
		}
	})
}

// 台帳から戻すのは「台帳で最新の端末ミラーの質問」が同じ質問のときだけ。別の質問に
// 答えた後に同じ質問が出たら新しい質問として開く（永久抑止にしない）。答えずに
// ネイティブの承認に置き換わった質問も、回答済みにはしない。
func TestLedgerRestoreDoesNotSwallowOtherQuestions(t *testing.T) {
	t.Run("別の質問に答えた後の同じ質問", func(t *testing.T) {
		s, ses := answeredVTMarkerSession(t, true)
		other := syntheticMarker(t, "Rename the table as well?")
		if !s.maybeBroadcastApprovalMarker(1, other, time.Now()) {
			t.Fatal("前提: 別の質問が開かない")
		}
		s.handleInput(proto.Message{SessionID: 1, Text: "2\r"})
		// ネイティブの承認に画面から答えて、メモリの枠を別の候補にする。
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		native := ses.pendingApproval
		s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: native.Sig, ApprovalCandidateKey: native.CandidateKey, ApprovalSourceEpoch: native.SourceEpoch})
		s.handleInput(proto.Message{SessionID: 1, Text: "続けてください\r"})
		if !reopenVTMarker(s, ses) {
			t.Fatalf("台帳の最新が別の質問なのに、同じ質問が開かない: %+v", ses.pendingApproval)
		}
	})
	t.Run("答えずに置き換わった質問", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		newApprovalLedger(t, s, 1, "grok")
		ses.vt = newVTBuffer(80, 8)
		ses.vt.scrollback = append([]string(nil), migrationMarkerLines...)
		if !reopenVTMarker(s, ses) {
			t.Fatal("前提: 端末ミラーのマーカーが開かない")
		}
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		s.handleNativeApprovalDetection(1, nil)
		if !reopenVTMarker(s, ses) {
			t.Fatalf("答えていないマーカーが、ネイティブの承認が消えた後に戻らない: %+v", ses.pendingApproval)
		}
	})
}

// マーカー無しの文章の質問（出力が落ち着いた時点で開く）も同じ持ち越しに乗る。
func TestAnsweredVTTextQuestionStaysAnsweredAcrossNativeApproval(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Deploy to staging finished.\r\nProceed with the production deploy? (Y:1/N:0)\r\n"))
	settle := func() {
		ses.Activity.OutputIdle = false
		ses.lastOutputAt = time.Now().Add(-time.Minute)
		s.evaluateIdle()
	}

	settle()
	if record := ses.pendingApproval; !record.isMarker() || record.Kind != approvalKindPlainYesNo {
		t.Fatalf("前提: 文章の質問が開かない: %+v", record)
	}
	s.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
	if ses.pendingApproval != nil {
		t.Fatalf("前提: 答えても記録が閉じない: %+v", ses.pendingApproval)
	}

	s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
	if !ses.pendingApproval.isNative() {
		t.Fatalf("前提: ネイティブの承認が開かない: %+v", ses.pendingApproval)
	}
	s.handleNativeApprovalDetection(1, nil)
	settle()
	if record := ses.pendingApproval; record.isMarker() {
		t.Fatalf("回答済みの文章の質問が開き直った: %+v", record)
	}
}

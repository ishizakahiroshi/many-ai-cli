package hub

import "testing"

func TestPTYInputAckRemovesInflight(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	wc := &wrapperConn{}
	ses.inflightInput = map[int64]inflightInput{
		7: {data: "answer\r", conn: wc},
	}

	s.handlePTYInputAck(wc, 1, 7)

	s.sessionsMu.Lock()
	_, stillInflight := ses.inflightInput[7]
	ackCapable := ses.inputAckCapable
	s.sessionsMu.Unlock()
	if stillInflight {
		t.Fatal("acked input remained in-flight")
	}
	if !wc.inputAckSeen.Load() {
		t.Fatal("ack-capable connection was not marked")
	}
	if !ackCapable {
		t.Fatal("ack capability was not remembered on the session")
	}
}

func TestDeferInflightForResendOnAckCapableDisconnect(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	wc := &wrapperConn{}
	wc.inputAckSeen.Store(true)
	other := &wrapperConn{}
	ses.inflightInput = map[int64]inflightInput{
		8:  {data: "third", conn: wc},
		2:  {data: "first", conn: wc},
		5:  {data: "second", conn: wc},
		11: {data: "other", conn: other},
	}
	s.pendingInput[1] = []string{"already queued"}

	s.sessionsMu.Lock()
	count, minSeq, maxSeq := s.deferInflightForResendLocked(1, wc)
	resend := append([]pendingFrame(nil), ses.resendInput...)
	pending := append([]string(nil), s.pendingInput[1]...)
	_, otherStillInflight := ses.inflightInput[11]
	s.sessionsMu.Unlock()

	if count != 3 || minSeq != 2 || maxSeq != 8 {
		t.Fatalf("deferred range = count:%d min:%d max:%d, want 3/2/8", count, minSeq, maxSeq)
	}
	// 再送は元の seq を保ったまま seq 昇順で並ぶ。seq を振り直すと wrapper 側の
	// 重複判定が効かず、既に PTY へ入った入力が二重に書かれる。
	wantSeq := []int64{2, 5, 8}
	wantData := []string{"first", "second", "third"}
	if len(resend) != len(wantSeq) {
		t.Fatalf("resend length = %d, want %d", len(resend), len(wantSeq))
	}
	for i := range wantSeq {
		if resend[i].seq != wantSeq[i] || resend[i].data != wantData[i] {
			t.Fatalf("resend[%d] = {seq:%d data:%q}, want {seq:%d data:%q}",
				i, resend[i].seq, resend[i].data, wantSeq[i], wantData[i])
		}
	}
	// pendingInput は再送キューと別管理なので触られない。
	if len(pending) != 1 || pending[0] != "already queued" {
		t.Fatalf("pendingInput = %#v, want untouched", pending)
	}
	if !otherStillInflight {
		t.Fatal("in-flight input from another connection was removed")
	}
}

// 接続単位のフラグが false でも、セッションが ack 対応と分かっていれば再送する。
// reattach 直後に 1 件も ack を受けないまま再び切れる窓を救うための回帰テスト
// （このケースを落とすと、修正前と同じく入力が消える）。
func TestDeferInflightForResendUsesSessionAckCapability(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.inputAckCapable = true
	wc := &wrapperConn{} // 新しい接続。まだ ack を 1 件も受けていない
	ses.inflightInput = map[int64]inflightInput{
		4: {data: "flushed\r", conn: wc},
	}

	s.sessionsMu.Lock()
	count, _, _ := s.deferInflightForResendLocked(1, wc)
	resend := append([]pendingFrame(nil), ses.resendInput...)
	s.sessionsMu.Unlock()

	if count != 1 {
		t.Fatalf("deferred %d inputs, want 1", count)
	}
	if len(resend) != 1 || resend[0].seq != 4 || resend[0].data != "flushed\r" {
		t.Fatalf("resend = %#v, want one frame with seq 4", resend)
	}
}

func TestDeferInflightForResendSkipsLegacyWrapper(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	wc := &wrapperConn{}
	ses.inflightInput = map[int64]inflightInput{
		3: {data: "already sent", conn: wc},
	}
	s.pendingInput[1] = []string{"existing"}

	s.sessionsMu.Lock()
	count, _, _ := s.deferInflightForResendLocked(1, wc)
	pending := append([]string(nil), s.pendingInput[1]...)
	resend := len(ses.resendInput)
	remaining := len(ses.inflightInput)
	s.sessionsMu.Unlock()

	if count != 0 {
		t.Fatalf("legacy wrapper deferred %d inputs, want 0", count)
	}
	if resend != 0 {
		t.Fatalf("legacy wrapper queued %d resend frames, want 0", resend)
	}
	if remaining != 0 {
		t.Fatalf("legacy wrapper left %d in-flight inputs, want 0", remaining)
	}
	if len(pending) != 1 || pending[0] != "existing" {
		t.Fatalf("pending after legacy disconnect = %#v, want existing only", pending)
	}
}

func TestResendFramesMergeKeepsSeqOrderAndBound(t *testing.T) {
	existing := []pendingFrame{{seq: 3, data: "c"}, {seq: 1, data: "a"}}
	incoming := []pendingFrame{{seq: 2, data: "b"}}

	got := mergeResendFrames(existing, incoming)

	wantSeq := []int64{1, 2, 3}
	if len(got) != len(wantSeq) {
		t.Fatalf("merged length = %d, want %d", len(got), len(wantSeq))
	}
	for i := range wantSeq {
		if got[i].seq != wantSeq[i] {
			t.Fatalf("merged[%d].seq = %d, want %d", i, got[i].seq, wantSeq[i])
		}
	}

	over := make([]pendingFrame, 0, maxInflightInputPerSession+5)
	for i := 1; i <= maxInflightInputPerSession+5; i++ {
		over = append(over, pendingFrame{seq: int64(i)})
	}
	bounded := mergeResendFrames(over, nil)
	if len(bounded) != maxInflightInputPerSession {
		t.Fatalf("bounded length = %d, want %d", len(bounded), maxInflightInputPerSession)
	}
	// 上限超過時は古い方（小さい seq）から捨てる。
	if bounded[0].seq != 6 {
		t.Fatalf("bounded[0].seq = %d, want 6", bounded[0].seq)
	}
}

func TestInflightInputDropsOldestAtCapacity(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	wc := &wrapperConn{}
	for i := 0; i < maxInflightInputPerSession+5; i++ {
		if seq := s.reserveInflightInput(wc, 1, string(rune('a'+i))); seq == 0 {
			t.Fatalf("reserveInflightInput returned zero at index %d", i)
		}
	}

	s.sessionsMu.Lock()
	length := len(ses.inflightInput)
	_, oldestStillPresent := ses.inflightInput[1]
	_, newestPresent := ses.inflightInput[int64(maxInflightInputPerSession+5)]
	s.sessionsMu.Unlock()
	if length != maxInflightInputPerSession {
		t.Fatalf("in-flight length = %d, want %d", length, maxInflightInputPerSession)
	}
	if oldestStillPresent {
		t.Fatal("oldest in-flight input was not dropped")
	}
	if !newestPresent {
		t.Fatal("newest in-flight input was dropped")
	}
}

// 再送フレームは元の seq のまま in-flight へ戻る（新しい seq を振らない）。
func TestReadmitInflightInputKeepsSeq(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	wc := &wrapperConn{}
	ses.inputSeq = 40

	if !s.readmitInflightInput(wc, 1, 7, "resent\r") {
		t.Fatal("readmitInflightInput returned false for a registered session")
	}

	s.sessionsMu.Lock()
	item, ok := ses.inflightInput[7]
	seqCounter := ses.inputSeq
	s.sessionsMu.Unlock()
	if !ok || item.data != "resent\r" || item.conn != wc {
		t.Fatalf("in-flight[7] = %#v, want the resent frame bound to wc", item)
	}
	if seqCounter != 40 {
		t.Fatalf("inputSeq = %d, want 40 (resend must not consume a new seq)", seqCounter)
	}
}

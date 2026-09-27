package hub

// 保留中の承認の記録（approval_record.go）のテストと、ほかのテストが使う補助。
// fixture はすべて合成データ（実際のセッションログの本文を持ち込まない）。

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/notify"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionstore"
)

// syntheticNativeApproval は codex 形式のネイティブ承認を合成する。
func syntheticNativeApproval(question string) *nativeApproval {
	a := &nativeApproval{
		Kind:     "native",
		Question: question,
		Options: []proto.ApprovalOption{
			{Num: 1, Label: "Yes", SendText: "1\r"},
			{Num: 0, Label: "No", SendText: "0\r"},
		},
	}
	a.Sig = approvalCandidateKey("codex", a.Kind, a.Question, a.Options)
	return a
}

// syntheticMarker は 1 問の承認マーカーブロックを合成する。
func syntheticMarker(t *testing.T, question string) *approvalMarkerBlock {
	t.Helper()
	marker := extractApprovalMarkerBlock([]string{
		approvalMarkerOpen,
		"Q1 " + question,
		" 1. Yes (Recommended)",
		" 2. No",
		" N. User specifies",
		approvalMarkerClose,
	})
	if marker == nil {
		t.Fatal("syntheticMarker: ブロックを取り出せない")
	}
	return marker
}

// pendingApprovalRows は /api/approval-history?session_id=&state=pending を叩き、
// そのセッションの pending 行を返す（C3 の完了条件は API の見え方で確かめる）。
func pendingApprovalRows(t *testing.T, s *Server, liveID int) []sessionstore.ApprovalRow {
	t.Helper()
	s.cfg.Token = "test-token"
	r := httptest.NewRequest(http.MethodGet, "/api/approval-history?session_id="+strconv.Itoa(liveID)+"&state=pending&token=test-token", nil)
	r.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleApprovalHistory(w, r)
	if w.Code != http.StatusOK {
		t.Fatalf("approval-history status = %d: %s", w.Code, w.Body.String())
	}
	var body struct {
		Approvals []sessionstore.ApprovalRow `json:"approvals"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode approval-history: %v", err)
	}
	return body.Approvals
}

func ledgerRow(t *testing.T, store *sessionstore.Store, liveID int, sig string) sessionstore.ApprovalRow {
	t.Helper()
	rows, err := store.ApprovalsByLiveSession(liveID, 50, false)
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range rows {
		if row.Sig == sig {
			return row
		}
	}
	t.Fatalf("台帳に sig %q の行が無い（%d 行）", shortSig(sig), len(rows))
	return sessionstore.ApprovalRow{}
}

func countMessages(messages []proto.Message, typ string) int {
	n := 0
	for _, m := range messages {
		if m.Type == typ {
			n++
		}
	}
	return n
}

// testNativeRecord はネイティブの記録を作る。以前 nativeApprovalSig 等を直接書いていた
// テストの置き換え先で、候補の同一性が要らないテストは key を空、epoch を 0 にする。
func testNativeRecord(sig, candidateKey string, sourceEpoch uint64) *approvalRecord {
	return &approvalRecord{
		Sig:          sig,
		CandidateKey: candidateKey,
		SourceEpoch:  sourceEpoch,
		Origin:       approvalRecordOriginNative,
		Source:       approvalSourceGoVT,
		Kind:         "native",
	}
}

// nativeRecordSig はネイティブの記録が開いていればその sig を、無ければ空文字を返す。
func nativeRecordSig(ses *session) string {
	if ses.pendingApproval.isNative() {
		return ses.pendingApproval.Sig
	}
	return ""
}

func nativeRecordKey(ses *session) string {
	if ses.pendingApproval.isNative() {
		return ses.pendingApproval.CandidateKey
	}
	return ""
}

func nativeRecordEpoch(ses *session) uint64 {
	if ses.pendingApproval.isNative() {
		return ses.pendingApproval.SourceEpoch
	}
	return 0
}

// markerRecordSig / Key / Epoch はマーカーの記録が開いていればその値を、無ければゼロ値を返す。
func markerRecordSig(ses *session) string {
	if ses.pendingApproval.isMarker() {
		return ses.pendingApproval.Sig
	}
	return ""
}

func markerRecordKey(ses *session) string {
	if ses.pendingApproval.isMarker() {
		return ses.pendingApproval.CandidateKey
	}
	return ""
}

func markerRecordEpoch(ses *session) uint64 {
	if ses.pendingApproval.isMarker() {
		return ses.pendingApproval.SourceEpoch
	}
	return 0
}

// ---- 子 plan C1 の C2: 記録が作られる ----

// 3 つの供給元（ネイティブ検出・端末ミラーのマーカー・トランスクリプトのマーカー）の
// どれからでも、同じ記録型に同一性・供給元・本文・検出時刻が入る。
func TestApprovalRecordOpensFromEverySource(t *testing.T) {
	t.Run("native", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "codex")
		approval := syntheticNativeApproval("Run git status?")
		s.handleNativeApprovalDetection(1, approval)
		r := ses.pendingApproval
		if !r.isNative() || r.Source != approvalSourceGoVT || r.Kind != "native" || r.Sig != approval.Sig ||
			r.CandidateKey == "" || r.CandidateShape == "" || r.SourceEpoch != 1 {
			t.Fatalf("native record identity = %+v", r)
		}
		if r.Question != approval.Question || len(r.Options) != 2 || r.Block != "" || r.DetectedAt.IsZero() {
			t.Fatalf("native record body = %+v", r)
		}
	})
	t.Run("vt marker", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		marker := syntheticMarker(t, "この方針で進めますか?")
		if !s.maybeBroadcastApprovalMarker(1, marker, time.Now()) {
			t.Fatal("VT のマーカーが記録にならなかった")
		}
		r := ses.pendingApproval
		if !r.isMarker() || r.Source != approvalSourceGoVT || r.Kind != "marker" || r.Sig != marker.Sig ||
			r.CandidateKey == "" || r.SourceEpoch != 1 {
			t.Fatalf("vt marker record identity = %+v", r)
		}
		if r.Block != marker.Block || len(r.Options) != 0 || r.DetectedAt.IsZero() {
			t.Fatalf("vt marker record body = %+v（マーカーは原文だけを持つ）", r)
		}
	})
	t.Run("transcript marker", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "claude")
		s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
			{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
		}, false, time.Now())
		r := ses.pendingApproval
		if !r.isMarker() || r.Source != approvalSourceTranscript || r.Kind != "marker" || r.CandidateKey == "" ||
			!strings.Contains(r.Block, "Q1 この方針で進めますか?") {
			t.Fatalf("transcript marker record = %+v", r)
		}
	})
}

// 回答済みの候補（同じ candidateKey・同じ世代）では記録を作らない。
func TestApprovalRecordNotOpenedForAnsweredCandidate(t *testing.T) {
	t.Run("native", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "codex")
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		r := ses.pendingApproval
		s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: r.Sig, ApprovalCandidateKey: r.CandidateKey, ApprovalSourceEpoch: r.SourceEpoch})
		s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		if ses.pendingApproval != nil {
			t.Fatalf("回答済みの候補で記録が開いた: %+v", *ses.pendingApproval)
		}
	})
	t.Run("marker", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		marker := syntheticMarker(t, "この方針で進めますか?")
		s.maybeBroadcastApprovalMarker(1, marker, time.Now())
		r := ses.pendingApproval
		s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: "browser-side-sig", ApprovalCandidateKey: r.CandidateKey, ApprovalSourceEpoch: r.SourceEpoch})
		if s.maybeBroadcastApprovalMarker(1, marker, time.Now()) || ses.pendingApproval != nil {
			t.Fatalf("回答済みのマーカーで記録が開いた: %+v", ses.pendingApproval)
		}
	})
}

// 新しい候補が来たら古い記録は上書きで閉じ、台帳の古い行は resolved になる。
// 新しい行は pending のまま（古い行の resolved を新しい行の INSERT より先に書く）。
func TestApprovalRecordReplacedByNewCandidate(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	store := newApprovalLedger(t, s, 1, "codex")
	first := syntheticNativeApproval("Run git status?")
	second := syntheticNativeApproval("Run git diff?")
	s.handleNativeApprovalDetection(1, first)
	s.handleNativeApprovalDetection(1, second)
	if got := nativeRecordSig(ses); got != second.Sig {
		t.Fatalf("native record sig = %q, want the newer candidate", got)
	}
	if row := ledgerRow(t, store, 1, first.Sig); row.State != "resolved" {
		t.Fatalf("上書きされた行 = %q, want resolved", row.State)
	}
	if row := ledgerRow(t, store, 1, second.Sig); row.State != "pending" {
		t.Fatalf("新しい行 = %q, want pending", row.State)
	}

	// トランスクリプトのマーカーは、開いているネイティブの記録も上書きする。
	marker := syntheticMarker(t, "方針を切り替えますか?")
	s.scanTranscriptApprovalMarkers(1, "codex", "rollout.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: marker.Block},
	}, false, time.Now())
	if markerRecordSig(ses) != marker.Sig || ses.pendingApproval.Source != approvalSourceTranscript {
		t.Fatalf("トランスクリプトのマーカーが記録を上書きしていない: %+v", ses.pendingApproval)
	}
	if row := ledgerRow(t, store, 1, second.Sig); row.State != "resolved" {
		t.Fatalf("上書きされたネイティブの行 = %q, want resolved", row.State)
	}
	pending := pendingApprovalRows(t, s, 1)
	if len(pending) != 1 || pending[0].Sig != marker.Sig {
		t.Fatalf("pending 行 = %+v, want いま開いているマーカーの 1 行だけ", pending)
	}
}

// 端末ミラーから読んだマーカーは、画面に出ているネイティブの記録を上書きしない
// （approval_record.go 冒頭の例外）。上書きすると、古いマーカーとネイティブの承認が
// 出力のたびに記録を奪い合う。
func TestVTMarkerDoesNotReplaceVisibleNativeRecord(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	sent := captureUIBroadcasts(s)
	native := syntheticNativeApproval("Run git status?")
	s.handleNativeApprovalDetection(1, native)
	epoch := ses.approvalSourceEpoch
	stale := syntheticMarker(t, "前の質問ですか?")

	for i := 0; i < 3; i++ {
		if s.maybeBroadcastApprovalMarker(1, stale, time.Now()) {
			t.Fatalf("%d 回目: 画面に出ているネイティブの記録を VT のマーカーが上書きした", i+1)
		}
		s.handleNativeApprovalDetection(1, native)
	}
	if nativeRecordSig(ses) != native.Sig || ses.approvalSourceEpoch != epoch {
		t.Fatalf("記録か世代が動いた: sig=%q epoch=%d want %d", nativeRecordSig(ses), ses.approvalSourceEpoch, epoch)
	}
	if got := len(approvalStateOpens(sent(), 1)); got != 1 {
		t.Fatalf("開く approval_state %d 件, want ネイティブの 1 件だけ（奪い合いが起きていない）", got)
	}

	// ネイティブが 1 回でも見えなくなったら、画面に出ているマーカーを優先する。
	s.handleNativeApprovalDetection(1, nil)
	if !s.maybeBroadcastApprovalMarker(1, stale, time.Now()) || markerRecordSig(ses) != stale.Sig {
		t.Fatalf("消えかけのネイティブの記録がマーカーを止め続けている: %+v", ses.pendingApproval)
	}

	// トランスクリプトのマーカーは一度きりの観測なので、この例外の対象にしない。
	s2 := newTestServer()
	claude := registerTestSession(s2, 2, "claude")
	s2.handleNativeApprovalDetection(2, syntheticNativeApproval("Run git status?"))
	s2.scanTranscriptApprovalMarkers(2, "claude", "transcript.jsonl", []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
	}, false, time.Now())
	if markerRecordKey(claude) == "" {
		t.Fatal("トランスクリプトのマーカーがネイティブの記録に止められた（一度きりの観測が失われる）")
	}
}

// ---- 子 plan C1 の C3: 閉じ方を Hub に揃える ----

// approvalStateCloses は配信された approval_state のうち「閉じる」だけを返す。
func approvalStateCloses(messages []proto.Message, sessionID int) []proto.ApprovalRecordClose {
	var out []proto.ApprovalRecordClose
	for _, m := range messages {
		if m.Type == "approval_state" && m.SessionID == sessionID && m.ApprovalState != nil && m.ApprovalState.Close != nil {
			out = append(out, *m.ApprovalState.Close)
		}
	}
	return out
}

func approvalStateOpens(messages []proto.Message, sessionID int) []proto.ApprovalRecord {
	var out []proto.ApprovalRecord
	for _, m := range messages {
		if m.Type == "approval_state" && m.SessionID == sessionID && m.ApprovalState != nil && m.ApprovalState.Open != nil {
			out = append(out, *m.ApprovalState.Open)
		}
	}
	return out
}

// assertSingleClose は、閉じる approval_state が理由付きで 1 回だけ出たことを確かめる（子 C2 の C1）。
func assertSingleClose(t *testing.T, sent func() []proto.Message, sessionID int, sig, reason string) {
	t.Helper()
	closes := approvalStateCloses(sent(), sessionID)
	if len(closes) != 1 || closes[0].Reason != reason || closes[0].Sig != sig {
		t.Fatalf("閉じる approval_state = %+v, want sig %q・理由 %q の 1 件", closes, shortSig(sig), reason)
	}
}

// assertApprovalRecordClosed は、記録が閉じ、台帳の行が resolved で、
// /api/approval-history?state=pending がそのセッションについて何も返さないことを確かめる。
func assertApprovalRecordClosed(t *testing.T, s *Server, ses *session, store *sessionstore.Store, liveID int, sig, wantAnswer string) {
	t.Helper()
	s.sessionsMu.Lock()
	record := ses.pendingApproval
	s.sessionsMu.Unlock()
	if record != nil {
		t.Fatalf("記録が閉じていない: %+v", *record)
	}
	row := ledgerRow(t, store, liveID, sig)
	if row.State != "resolved" || row.SelectedText != wantAnswer {
		t.Fatalf("台帳の行 = state %q selected %q, want resolved / %q", row.State, row.SelectedText, wantAnswer)
	}
	if pending := pendingApprovalRows(t, s, liveID); len(pending) != 0 {
		t.Fatalf("閉じた後も pending 行が %d 件ある: %+v", len(pending), pending)
	}
}

// openNativeRecordWithLedger は台帳付きのセッションでネイティブの記録を開き、
// 以後の配信を受け取る画面を 1 つ登録する。
func openNativeRecordWithLedger(t *testing.T, provider string) (*Server, *session, *sessionstore.Store, *approvalRecord, func() []proto.Message) {
	t.Helper()
	s := newTestServer()
	ses := registerTestSession(s, 1, provider)
	store := newApprovalLedger(t, s, 1, provider)
	s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
	if ses.pendingApproval == nil {
		t.Fatal("前提が違う: ネイティブの記録が開いていない")
	}
	if len(pendingApprovalRows(t, s, 1)) != 1 {
		t.Fatal("前提が違う: 台帳に pending 行が無い")
	}
	return s, ses, store, ses.pendingApproval, captureUIBroadcasts(s)
}

// 子 plan C1 の前提の表にある閉じ方すべてで、記録が閉じて台帳が resolved になる。
func TestApprovalRecordClosePathsResolveLedger(t *testing.T) {
	t.Run("画面での回答（approval_consumed）", func(t *testing.T) {
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: r.Sig, ApprovalCandidateKey: r.CandidateKey, ApprovalSourceEpoch: r.SourceEpoch, SentText: "1"})
		assertApprovalRecordClosed(t, s, ses, store, 1, r.Sig, "1")
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseAnswered)
	})

	t.Run("one-tap・自動承認・一括承認", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "codex")
		store := newApprovalLedger(t, s, 1, "codex")
		sent := captureUIBroadcasts(s)
		lines := []string{
			"Command requires approval",
			"Run: git status",
			"",
			"❯ Yes (y)",
			"  Yes, and don't ask again for this command (p)",
			"  No (n)",
			"  Cancel (esc)",
		}
		ses.vt = newVTBuffer(120, 30)
		ses.vt.Write([]byte(strings.Join(lines, "\r\n")))
		wc := &wrapperConn{sendFunc: func(any) error { return nil }}
		s.sessionsMu.Lock()
		s.wrappers[1] = wc
		s.sessionsMu.Unlock()
		s.handleNativeApprovalDetection(1, detectNativeApproval("codex", ses.vt.Lines()))
		r := ses.pendingApproval
		if r == nil {
			t.Fatal("前提が違う: ネイティブの記録が開いていない")
		}
		result, err := s.sendNativeApprovalAction(nativeApprovalActionRequest{
			sessionID: 1, approvalSig: r.Sig, expectedCandidateKey: r.CandidateKey,
			expectedSourceEpoch: r.SourceEpoch, expectedWrapper: wc, action: oneTapReject,
		})
		if err != nil {
			t.Fatalf("sendNativeApprovalAction = %v", err)
		}
		if !s.commitNativeApprovalAction(result) {
			t.Fatal("commitNativeApprovalAction = false")
		}
		assertApprovalRecordClosed(t, s, ses, store, 1, r.Sig, result.input)
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseAnswered)
	})

	t.Run("トランスクリプトの user メッセージ", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "claude")
		store := newApprovalLedger(t, s, 1, "claude")
		sent := captureUIBroadcasts(s)
		s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
			{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
		}, false, time.Now())
		sig := markerRecordSig(ses)
		s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
			{Role: "user", Kind: "text", Text: "2"},
		}, false, time.Now())
		assertApprovalRecordClosed(t, s, ses, store, 1, sig, "2")
		assertSingleClose(t, sent, 1, sig, approvalCloseAnsweredTerminal)
	})

	t.Run("確定したユーザーターン（端末ミラーのマーカー）", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		store := newApprovalLedger(t, s, 1, "grok")
		sent := captureUIBroadcasts(s)
		marker := syntheticMarker(t, "この方針で進めますか?")
		s.maybeBroadcastApprovalMarker(1, marker, time.Now())
		s.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
		assertApprovalRecordClosed(t, s, ses, store, 1, marker.Sig, "1")
		assertSingleClose(t, sent, 1, marker.Sig, approvalCloseAnsweredTerminal)
	})

	t.Run("ネイティブの消失", func(t *testing.T) {
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		for i := 0; i < nativeApprovalClearMissLimit; i++ {
			s.handleNativeApprovalDetection(1, nil)
		}
		assertApprovalRecordClosed(t, s, ses, store, 1, r.Sig, "")
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseVanished)
	})

	t.Run("別の候補による上書き", func(t *testing.T) {
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		next := syntheticNativeApproval("Run git diff?")
		s.handleNativeApprovalDetection(1, next)
		if row := ledgerRow(t, store, 1, r.Sig); row.State != "resolved" {
			t.Fatalf("上書きされた行 = %q, want resolved", row.State)
		}
		pending := pendingApprovalRows(t, s, 1)
		if len(pending) != 1 || pending[0].Sig != next.Sig || nativeRecordSig(ses) != next.Sig {
			t.Fatalf("pending 行 = %+v, want 新しい候補の 1 行だけ", pending)
		}
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseSuperseded)
	})

	t.Run("セッション終了（dismiss）", func(t *testing.T) {
		withApprovalTestHome(t)
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 1})
		assertApprovalRecordClosed(t, s, ses, store, 1, r.Sig, "")
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseSessionEnd)
	})

	t.Run("セッション終了（wrapper の切断）", func(t *testing.T) {
		withApprovalTestHome(t)
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		disconnectTestWrapper(t, s, 1)
		assertApprovalRecordClosed(t, s, ses, store, 1, r.Sig, "")
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseSessionEnd)
	})

	t.Run("history reset", func(t *testing.T) {
		s, ses, store, r, sent := openNativeRecordWithLedger(t, "codex")
		s.handleHistoryReset(proto.Message{Type: "session_history_reset", SessionID: 1})
		if ses.pendingApproval != nil {
			t.Fatalf("history reset の後も記録が残っている: %+v", *ses.pendingApproval)
		}
		// history reset は台帳の行ごと消す。
		if rows, err := store.ApprovalsByLiveSession(1, 50, false); err != nil || len(rows) != 0 {
			t.Fatalf("history reset の後の台帳 = %d 行 (err=%v), want 0", len(rows), err)
		}
		assertSingleClose(t, sent, 1, r.Sig, approvalCloseHistoryReset)
	})
}

// ---- 子 plan C2 の C1: 記録を運ぶメッセージ ----

// 記録ができたとき開く approval_state が 1 回だけ出て、同じ候補の再検出では出ない。
// 版番号はセッションごとに開閉のたびに 1 ずつ進む。
func TestApprovalStateOpensOnceAndVersionsIncrease(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	other := registerTestSession(s, 2, "grok")
	sent := captureUIBroadcasts(s)

	first := syntheticNativeApproval("Run git status?")
	s.handleNativeApprovalDetection(1, first)
	s.handleNativeApprovalDetection(1, first)
	opens := approvalStateOpens(sent(), 1)
	if len(opens) != 1 || opens[0].Sig != first.Sig || opens[0].Origin != approvalRecordOriginNative ||
		opens[0].Question != first.Question || len(opens[0].Options) != 2 || opens[0].Summary == nil {
		t.Fatalf("開く approval_state = %+v, want ネイティブの本文付きで 1 件", opens)
	}

	second := syntheticNativeApproval("Run git diff?")
	s.handleNativeApprovalDetection(1, second)
	s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: second.Sig, ApprovalCandidateKey: ses.pendingApproval.CandidateKey, ApprovalSourceEpoch: ses.pendingApproval.SourceEpoch})

	// 別のセッションの版番号は別に数える。
	s.maybeBroadcastApprovalMarker(2, syntheticMarker(t, "進めますか?"), time.Now())

	var versions []uint64
	var kinds []string
	for _, m := range sent() {
		if m.Type != "approval_state" || m.SessionID != 1 {
			continue
		}
		versions = append(versions, m.ApprovalState.Version)
		if m.ApprovalState.Open != nil {
			kinds = append(kinds, "open")
		} else {
			kinds = append(kinds, "close:"+m.ApprovalState.Close.Reason)
		}
	}
	wantKinds := []string{"open", "close:" + approvalCloseSuperseded, "open", "close:" + approvalCloseAnswered}
	if strings.Join(kinds, ",") != strings.Join(wantKinds, ",") {
		t.Fatalf("approval_state の並び = %v, want %v", kinds, wantKinds)
	}
	for i, v := range versions {
		if v != uint64(i+1) {
			t.Fatalf("版番号 = %v, want 1 から 1 ずつ", versions)
		}
	}
	if other.approvalStateVersion != 1 {
		t.Fatalf("別のセッションの版番号 = %d, want 1（セッションごとに数える）", other.approvalStateVersion)
	}
	opensMarker := approvalStateOpens(sent(), 2)
	if len(opensMarker) != 1 || opensMarker[0].Origin != approvalRecordOriginMarker || opensMarker[0].Block == "" ||
		len(opensMarker[0].Options) != 0 || opensMarker[0].Summary != nil {
		t.Fatalf("マーカーの開く approval_state = %+v, want 原文だけ", opensMarker)
	}
}

// reattach でセッションを作り直しても、記録と版番号は巻き戻さない。0 から数え直すと、
// 画面は手元より古い版を捨てるので、reattach 後の開閉がすべて捨てられる。
func TestApprovalStateVersionSurvivesReattach(t *testing.T) {
	withApprovalTestHome(t)
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.CWD = "/tmp"
	record := testNativeRecord("sig-kept", "key-kept", 1)
	s.sessionsMu.Lock()
	ses.pendingApproval = record
	ses.approvalStateVersion = 7
	s.wrappers[1] = &wrapperConn{pid: 123}
	s.sessionsMu.Unlock()

	loopDone := make(chan struct{})
	server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		defer close(loopDone)
		var req proto.Message
		if err := websocket.JSON.Receive(conn, &req); err != nil {
			return
		}
		s.reattachLoop(conn, req)
	}))
	defer server.Close()
	parsed, err := url.Parse(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	conn, err := websocket.Dial("ws"+strings.TrimPrefix(server.URL, "http"), "", "http://"+parsed.Host)
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	// 切断後の後始末（承認ルールの回収など）はテスト用 HOME の下を触る。後始末が
	// 終わる前に戻ると、t.TempDir の削除と競合して "directory is not empty" で落ちる。
	defer func() {
		_ = conn.Close()
		select {
		case <-loopDone:
		case <-time.After(5 * time.Second):
			t.Error("reattach の後始末が終わらない")
		}
	}()
	if err := websocket.JSON.Send(conn, proto.Message{
		Type: "reattach", SessionID: 1, Provider: "codex", CWD: "/tmp", PID: 123, Cols: 80, Rows: 24, UsageProbe: true,
	}); err != nil {
		t.Fatal(err)
	}
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	for {
		var frame proto.Message
		if err := websocket.JSON.Receive(conn, &frame); err != nil {
			t.Fatalf("reattach_ack が届かない: %v", err)
		}
		if frame.Type == "reattach_ack" {
			break
		}
	}
	s.sessionsMu.Lock()
	current := s.sessions[1]
	gotVersion, gotRecord := current.approvalStateVersion, current.pendingApproval
	s.sessionsMu.Unlock()
	if current == ses {
		t.Fatal("前提が違う: reattach でセッションが作り直されていない")
	}
	if gotVersion != 7 || gotRecord != record {
		t.Fatalf("reattach 後 = version %d record %+v, want 7 と同じ記録", gotVersion, gotRecord)
	}
}

// ---- 子 plan C2 の C2: 接続した画面にだけ、いまの記録をまとめて送る ----

// captureSnapshotUI は s.uis に載せない画面を 1 つ作り、送られたメッセージを返す関数を返す
// （接続したばかりの画面の代わり）。
func captureSnapshotUI() (*uiConn, func() []proto.Message) {
	uc := newUIConn(&websocket.Conn{})
	var got []proto.Message
	uc.sendFunc = func(m any) error {
		if msg, ok := m.(proto.Message); ok {
			got = append(got, msg)
		}
		return nil
	}
	return uc, func() []proto.Message { return append([]proto.Message(nil), got...) }
}

func approvalSnapshotOf(t *testing.T, messages []proto.Message) []proto.ApprovalSessionState {
	t.Helper()
	var snaps [][]proto.ApprovalSessionState
	for _, m := range messages {
		if m.Type == "approval_snapshot" {
			snaps = append(snaps, m.ApprovalSnapshot)
		}
	}
	if len(snaps) != 1 {
		t.Fatalf("approval_snapshot = %d 通, want 1", len(snaps))
	}
	return snaps[0]
}

func TestApprovalSnapshotGoesOnlyToConnectingUI(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "codex")
	registerTestSession(s, 2, "grok")
	probe := registerTestSession(s, 3, "claude")
	probe.UsageProbe = true
	s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
	existing := captureUIBroadcasts(s)

	uc, got := captureSnapshotUI()
	s.sendSnapshot(uc)

	snapshot := approvalSnapshotOf(t, got())
	if len(snapshot) != 2 || snapshot[0].SessionID != 1 || snapshot[1].SessionID != 2 {
		t.Fatalf("approval_snapshot = %+v, want セッション 1 と 2（usage probe は除く）", snapshot)
	}
	if snapshot[0].Record == nil || snapshot[0].Record.Origin != approvalRecordOriginNative || snapshot[0].Version != 1 {
		t.Fatalf("セッション 1 = %+v, want 開いているネイティブの記録と版 1", snapshot[0])
	}
	if snapshot[1].Record != nil || snapshot[1].Version != 0 {
		t.Fatalf("セッション 2 = %+v, want 記録なし", snapshot[1])
	}
	if n := countMessages(existing(), "approval_snapshot"); n != 0 {
		t.Fatalf("前から接続している画面へ approval_snapshot が %d 通届いた", n)
	}

	// 記録が閉じた後に接続した画面には「無い」として届く。
	r := s.sessions[1].pendingApproval
	s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: r.Sig, ApprovalCandidateKey: r.CandidateKey, ApprovalSourceEpoch: r.SourceEpoch})
	uc2, got2 := captureSnapshotUI()
	s.sendSnapshot(uc2)
	after := approvalSnapshotOf(t, got2())
	if after[0].SessionID != 1 || after[0].Record != nil || after[0].Version != 2 {
		t.Fatalf("閉じた後のセッション 1 = %+v, want 記録なし・版 2", after[0])
	}
}

// Hub を再起動すると記録はメモリごと消える。wrapper の再接続で replay を再評価すると
// （ネイティブ・端末ミラーのマーカー）、トランスクリプトの prime で最後のメッセージが
// マーカーなら、記録が作り直され、開く approval_state が届く。
func TestApprovalRecordRebuiltAfterHubRestart(t *testing.T) {
	t.Run("ネイティブ（replay の再評価）", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "codex")
		sent := captureUIBroadcasts(s)
		ses.vt = newVTBuffer(120, 30)
		ses.vt.Write([]byte(strings.Join([]string{
			"Command requires approval",
			"Run: git status",
			"",
			"❯ Yes (y)",
			"  Yes, and don't ask again for this command (p)",
			"  No (n)",
			"  Cancel (esc)",
		}, "\r\n")))
		s.evaluateReplayApproval(1)
		if !ses.pendingApproval.isNative() || len(approvalStateOpens(sent(), 1)) != 1 {
			t.Fatalf("replay でネイティブの記録が戻らない: %+v", ses.pendingApproval)
		}
	})
	t.Run("端末ミラーのマーカー（replay の再評価）", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		sent := captureUIBroadcasts(s)
		ses.vt = newVTBuffer(60, 20)
		ses.vt.Write([]byte(strings.ReplaceAll(syntheticMarker(t, "進めますか?").Block, "\n", "\r\n")))
		s.evaluateReplayApproval(1)
		if !ses.pendingApproval.isMarker() || len(approvalStateOpens(sent(), 1)) != 1 {
			t.Fatalf("replay でマーカーの記録が戻らない: %+v", ses.pendingApproval)
		}
	})
	t.Run("トランスクリプトのマーカー（prime）", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "claude")
		sent := captureUIBroadcasts(s)
		s.scanTranscriptApprovalMarkers(1, "claude", "transcript.jsonl", []agentChatMessage{
			{Role: "user", Kind: "text", Text: "進めて"},
			{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
		}, true, time.Now())
		if !ses.pendingApproval.isMarker() || ses.pendingApproval.Source != approvalSourceTranscript ||
			len(approvalStateOpens(sent(), 1)) != 1 {
			t.Fatalf("prime でトランスクリプトの記録が戻らない: %+v", ses.pendingApproval)
		}
	})
}

// 自動承認した記録は、開いたことを知らせない。閉じるだけが届き、
// 画面は知らない記録の「閉じる」として版番号を進めるだけになる。
func TestApprovalStateAutoApprovedRecordSendsOnlyClose(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	sent := captureUIBroadcasts(s)
	s.sessionsMu.Lock()
	ses.pendingApproval = testNativeRecord("sig-auto", "key-auto", 1)
	closure := closeApprovalRecordLocked(ses, 1, approvalCloseAnswered, time.Now())
	s.sessionsMu.Unlock()
	s.finishApprovalRecordClosures(closure)
	if opens := approvalStateOpens(sent(), 1); len(opens) != 0 {
		t.Fatalf("開く approval_state = %+v, want 0", opens)
	}
	assertSingleClose(t, sent, 1, "sig-auto", approvalCloseAnswered)
}

// disconnectTestWrapper は wrapper の WebSocket をつないで切り、Hub 側の後始末
// （wrapperMessageLoop の末尾）を最後まで走らせる。
func disconnectTestWrapper(t *testing.T, s *Server, id int) {
	t.Helper()
	done := make(chan struct{})
	server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		wc := newWrapperConn(conn)
		s.sessionsMu.Lock()
		s.wrappers[id] = wc
		s.sessionsMu.Unlock()
		s.wrapperMessageLoop(wc, id)
		close(done)
	}))
	defer server.Close()
	parsed, err := url.Parse(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	conn, err := websocket.Dial("ws"+strings.TrimPrefix(server.URL, "http"), "", "http://"+parsed.Host)
	if err != nil {
		t.Fatalf("dial test wrapper: %v", err)
	}
	_ = conn.Close()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("wrapper の後始末が終わらない")
	}
}

// 確定したユーザーターンでマーカーの記録を閉じた後に、画面の approval_consumed が
// 届いても何も壊れない（回答の Enter は approval_consumed より先に届く）。
func TestSubmittedTurnClosesMarkerAndLateConsumedIsIdempotent(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	store := newApprovalLedger(t, s, 1, "grok")
	sent := captureUIBroadcasts(s)
	marker := syntheticMarker(t, "この方針で進めますか?")
	s.maybeBroadcastApprovalMarker(1, marker, time.Now())
	key, epoch := markerRecordKey(ses), markerRecordEpoch(ses)

	s.handleInput(proto.Message{SessionID: 1, Text: "1\r"})
	if ses.pendingApproval != nil {
		t.Fatal("確定したユーザーターンでマーカーの記録が閉じていない")
	}
	if ses.approvalConsumedCandidateKey != key || ses.approvalConsumedEpoch != epoch || !ses.approvalEpochPending || ses.approvalSourceEpoch != epoch {
		t.Fatalf("回答済み = (%q, %d, pending=%v, epoch=%d), want (%q, %d, true, %d)（世代は進めない）",
			ses.approvalConsumedCandidateKey, ses.approvalConsumedEpoch, ses.approvalEpochPending, ses.approvalSourceEpoch, key, epoch, epoch)
	}
	// 閉じたことは approval_state で全部の画面へ届く（別の画面のパネルもこれで閉じる）。
	closes := approvalStateCloses(sent(), 1)
	if len(closes) != 1 || closes[0].Origin != approvalRecordOriginMarker || closes[0].CandidateKey != key ||
		closes[0].SourceEpoch != epoch || closes[0].Reason != approvalCloseAnsweredTerminal {
		t.Fatalf("閉じる approval_state = %+v, want マーカーの記録を answered_terminal で 1 件", closes)
	}

	s.markNativeApprovalConsumed(proto.Message{
		Type: "approval_consumed", SessionID: 1,
		ApprovalSig: "browser-side-sig", ApprovalCandidateKey: key, ApprovalSourceEpoch: epoch,
		ApprovalSource: "hub_marker", SentText: "1",
	})
	if ses.pendingApproval != nil || ses.approvalConsumedCandidateKey != key || ses.approvalConsumedEpoch != epoch ||
		!ses.approvalEpochPending || ses.approvalSourceEpoch != epoch {
		t.Fatalf("遅れて届いた approval_consumed で状態が変わった: record=%+v consumed=(%q,%d) pending=%v epoch=%d",
			ses.pendingApproval, ses.approvalConsumedCandidateKey, ses.approvalConsumedEpoch, ses.approvalEpochPending, ses.approvalSourceEpoch)
	}
	if n := len(approvalStateCloses(sent(), 1)); n != 1 {
		t.Fatalf("閉じる approval_state = %d 件, want 1（遅れた approval_consumed で閉じ直さない）", n)
	}
	if row := ledgerRow(t, store, 1, marker.Sig); row.State != "resolved" || row.SelectedText != "1" {
		t.Fatalf("台帳の行 = %+v", row)
	}
	// 画面に残っている同じブロックを読み直しても、記録は開き直さない。
	if s.maybeBroadcastApprovalMarker(1, marker, time.Now()) {
		t.Fatal("回答済みのマーカーが開き直した")
	}
}

// Hub から送る入力（オーケストレーションの send など）も確定したユーザーターンとして
// マーカーの記録を閉じる。ネイティブの記録はキー 1 つで答えるので、文章の送信では閉じない。
func TestSubmittedTurnClosesOnlyMarkerRecords(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	s.maybeBroadcastApprovalMarker(1, syntheticMarker(t, "この方針で進めますか?"), time.Now())
	s.injectRaw(1, bracketedPasteStart+"方針 A で進めてください"+bracketedPasteEnd+"\r")
	if ses.pendingApproval != nil {
		t.Fatal("Hub から送った確定入力でマーカーの記録が閉じていない")
	}

	native := registerTestSession(s, 2, "codex")
	s.handleNativeApprovalDetection(2, syntheticNativeApproval("Run git status?"))
	s.handleInput(proto.Message{SessionID: 2, Text: "hello\r"})
	if !native.pendingApproval.isNative() {
		t.Fatal("文章の送信でネイティブの記録が閉じた")
	}
	// 本文だけのフレーム（確定前）や、ペースト後の確定 \r 単独では数えない。
	marker := registerTestSession(s, 3, "grok")
	s.maybeBroadcastApprovalMarker(3, syntheticMarker(t, "次の質問ですか?"), time.Now())
	s.handleInput(proto.Message{SessionID: 3, Text: "typing"})
	s.handleInput(proto.Message{SessionID: 3, Text: "\r"})
	if !marker.pendingApproval.isMarker() {
		t.Fatal("確定していない入力でマーカーの記録が閉じた")
	}
}

// ---- 子 plan C2 の C3: 「保留中」と通知を記録から出す ----

// 記録が開いた時点で「保留中」が立ち、閉じた時点で下りる。画面の申告は要らない
// （このテストは画面を 1 つも登録していない状態で始める）。
func TestAwaitingFollowsApprovalRecord(t *testing.T) {
	for _, tc := range []struct {
		name     string
		provider string
		open     func(t *testing.T, s *Server) *approvalRecord
	}{
		{name: "native", provider: "codex", open: func(t *testing.T, s *Server) *approvalRecord {
			s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
			return s.sessions[1].pendingApproval
		}},
		{name: "vt marker", provider: "grok", open: func(t *testing.T, s *Server) *approvalRecord {
			s.maybeBroadcastApprovalMarker(1, syntheticMarker(t, "この方針で進めますか?"), time.Now())
			return s.sessions[1].pendingApproval
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newTestServer()
			ses := registerTestSession(s, 1, tc.provider)
			ses.lastOutputAt = time.Now()
			record := tc.open(t, s)
			if record == nil {
				t.Fatal("記録が開かなかった")
			}
			if ses.State != "waiting" || !ses.Activity.AwaitingApproval || !ses.Activity.AwaitingUser {
				t.Fatalf("記録を開いた直後: state=%q activity=%+v, want waiting", ses.State, ses.Activity)
			}
			// 出力が静まって idle の判定が走っても、記録がある間は下ろさない。
			s.evaluateIdle()
			if ses.State != "waiting" || !ses.Activity.AwaitingApproval {
				t.Fatalf("evaluateIdle の後: state=%q activity=%+v, want waiting", ses.State, ses.Activity)
			}

			s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: record.Sig, ApprovalCandidateKey: record.CandidateKey, ApprovalSourceEpoch: record.SourceEpoch})
			if ses.pendingApproval != nil || ses.Activity.AwaitingApproval || ses.Activity.AwaitingUser || ses.State == "waiting" {
				t.Fatalf("記録を閉じた後: record=%+v state=%q activity=%+v", ses.pendingApproval, ses.State, ses.Activity)
			}
		})
	}
}

// countingWebhook は承認の通知を受ける webhook を立て、届いた件数を返す関数と、
// その webhook へ送る Hub を返す。
func countingWebhook(t *testing.T) (*Server, func() int) {
	t.Helper()
	var mu sync.Mutex
	hits := 0
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		hits++
		mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
	}))
	t.Cleanup(srv.Close)
	s := newTestServer()
	s.notifyMgr = notify.New(notify.Config{
		Backends: []notify.BackendConfig{{Type: "webhook", URL: srv.URL}},
		Events:   []string{"approval"},
	}, nil)
	return s, func() int {
		mu.Lock()
		defer mu.Unlock()
		return hits
	}
}

// assertNotificationCount は非同期に届く通知が want 件になるのを待ち、そのあと少し待って
// 余分な通知が遅れて届かないことも確かめる。
func assertNotificationCount(t *testing.T, count func() int, want int) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for count() < want && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	time.Sleep(200 * time.Millisecond)
	if got := count(); got != want {
		t.Fatalf("承認の通知 = %d 件, want %d", got, want)
	}
}

// 承認の外部通知（ntfy / webhook）は記録が開いたときの 1 回だけ出る。同じ承認を
// 検出し直しても、waiting への遷移でも、もう 1 回は出ない。画面は 1 つも開いていない。
func TestApprovalNotificationOncePerRecord(t *testing.T) {
	for _, tc := range []struct {
		name     string
		provider string
		detect   func(t *testing.T, s *Server)
	}{
		{name: "native", provider: "codex", detect: func(t *testing.T, s *Server) {
			s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))
		}},
		{name: "vt marker", provider: "grok", detect: func(t *testing.T, s *Server) {
			s.maybeBroadcastApprovalMarker(1, syntheticMarker(t, "この方針で進めますか?"), time.Now())
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, count := countingWebhook(t)
			ses := registerTestSession(s, 1, tc.provider)
			ses.lastOutputAt = time.Now().Add(-time.Minute)

			for i := 0; i < 3; i++ {
				tc.detect(t, s)
				s.evaluateIdle()
			}
			if ses.pendingApproval == nil || ses.State != "waiting" {
				t.Fatalf("記録が開いていない: record=%+v state=%q", ses.pendingApproval, ses.State)
			}
			assertNotificationCount(t, count, 1)
		})
	}
}

// ユーザーのターンをはさんで同じ質問がもう一度来たら、それは別の記録なのでもう一度鳴る。
// 通知の ID を sig だけにすると、送信側の重複抑止（1 時間）に当たって鳴らない。
func TestApprovalNotificationFiresAgainForRepeatedQuestion(t *testing.T) {
	s, count := countingWebhook(t)
	ses := registerTestSession(s, 1, "codex")
	approval := syntheticNativeApproval("Run git status?")

	s.handleNativeApprovalDetection(1, approval)
	first := ses.pendingApproval
	s.markNativeApprovalConsumed(proto.Message{SessionID: 1, ApprovalSig: first.Sig, ApprovalCandidateKey: first.CandidateKey, ApprovalSourceEpoch: first.SourceEpoch})
	s.handleInput(proto.Message{SessionID: 1, Text: "次の作業をお願いします\r"})
	s.handleNativeApprovalDetection(1, syntheticNativeApproval("Run git status?"))

	second := ses.pendingApproval
	if second == nil || second.SourceEpoch == first.SourceEpoch {
		t.Fatalf("前提が違う: 2 回目の記録 = %+v（世代 %d のまま）", second, first.SourceEpoch)
	}
	assertNotificationCount(t, count, 2)
}

// ---- 子 plan C4 の C2: 抑止告知の「再検出」は Hub への問い直し（approval_resync）----

// registerCaptureUI は broadcast も受け取る画面を 1 つ登録し、その接続と、届いたメッセージを
// 返す関数を返す（captureUIBroadcasts と同じ sendFunc 差し替え。接続も要るので別に持つ）。
func registerCaptureUI(s *Server) (*uiConn, func() []proto.Message) {
	conn := &websocket.Conn{}
	uc := newUIConn(conn)
	var mu sync.Mutex
	var got []proto.Message
	uc.sendFunc = func(m any) error {
		if msg, ok := m.(proto.Message); ok {
			mu.Lock()
			got = append(got, msg)
			mu.Unlock()
		}
		return nil
	}
	s.sessionsMu.Lock()
	s.uis[conn] = uc
	s.sessionsMu.Unlock()
	return uc, func() []proto.Message {
		mu.Lock()
		defer mu.Unlock()
		return append([]proto.Message(nil), got...)
	}
}

func approvalStatesFor(messages []proto.Message, sessionID int) []proto.ApprovalState {
	var out []proto.ApprovalState
	for _, m := range messages {
		if m.Type == "approval_state" && m.SessionID == sessionID && m.ApprovalState != nil {
			out = append(out, *m.ApprovalState)
		}
	}
	return out
}

func TestApprovalResyncRepliesOnlyToTheAskingUI(t *testing.T) {
	t.Run("記録があるとき: 今の記録と版番号を問い直した画面にだけ送り直す", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "codex")
		ses.vt = newVTBuffer(120, 30)
		ses.vt.Write([]byte(strings.Join([]string{
			"Command requires approval",
			"Run: git status",
			"",
			"❯ Yes (y)",
			"  Yes, and don't ask again for this command (p)",
			"  No (n)",
			"  Cancel (esc)",
		}, "\r\n")))
		s.handleNativeApprovalDetection(1, detectNativeApproval("codex", ses.vt.Lines()))
		record := ses.pendingApproval
		if record == nil {
			t.Fatal("前提が違う: ネイティブの記録が開いていない")
		}
		version := ses.approvalStateVersion

		asker, askerGot := registerCaptureUI(s)
		_, otherGot := registerCaptureUI(s)
		s.handleApprovalResync(asker, 1)

		states := approvalStatesFor(askerGot(), 1)
		if len(states) != 1 || states[0].Open == nil || states[0].Close != nil {
			t.Fatalf("問い直した画面への返事 = %+v, want 今の記録を 1 通", states)
		}
		if states[0].Version != version || states[0].Open.CandidateKey != record.CandidateKey || states[0].Open.SourceEpoch != record.SourceEpoch {
			t.Fatalf("返事 = version %d / %+v, want version %d / %s#%d", states[0].Version, states[0].Open, version, record.CandidateKey, record.SourceEpoch)
		}
		if got := otherGot(); len(got) != 0 {
			t.Fatalf("ほかの画面に %d 通届いた: %+v（状態が変わっていないので送らない）", len(got), got)
		}
		if ses.approvalStateVersion != version || ses.pendingApproval != record {
			t.Fatal("問い直しで記録か版番号が変わった（同じ候補を二度開かない）")
		}
	})

	t.Run("評価し直して記録ができたとき: 開いたことは全画面へ、返事は問い直した画面にだけ", func(t *testing.T) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "grok")
		ses.vt = newVTBuffer(120, 30)
		ses.vt.Write([]byte(strings.Join([]string{
			approvalMarkerOpen,
			"Q1 この方針で進めますか?",
			" 1. Yes (Recommended)",
			" 2. No",
			" N. User specifies",
			approvalMarkerClose,
		}, "\r\n")))
		asker, askerGot := registerCaptureUI(s)
		_, otherGot := registerCaptureUI(s)

		s.handleApprovalResync(asker, 1)

		if ses.pendingApproval == nil || !ses.pendingApproval.isMarker() {
			t.Fatalf("問い直しで端末ミラーのマーカーが記録にならなかった: %+v", ses.pendingApproval)
		}
		if opens := approvalStateOpens(otherGot(), 1); len(opens) != 1 {
			t.Fatalf("ほかの画面への開く = %d 通, want 1（記録ができたのは状態の変化なので全画面へ）", len(opens))
		}
		if got := approvalStatesFor(otherGot(), 1); len(got) != 1 {
			t.Fatalf("ほかの画面への approval_state = %d 通, want 1（返事は届かない）", len(got))
		}
		states := approvalStatesFor(askerGot(), 1)
		if len(states) != 2 || states[1].Version != states[0].Version || states[1].Open == nil {
			t.Fatalf("問い直した画面 = %+v, want 開く + 同じ版の返事", states)
		}
	})

	t.Run("記録が無いとき: Open も Close も空の返事を問い直した画面にだけ送る", func(t *testing.T) {
		s := newTestServer()
		registerTestSession(s, 1, "codex")
		asker, askerGot := registerCaptureUI(s)
		_, otherGot := registerCaptureUI(s)
		s.handleApprovalResync(asker, 1)
		states := approvalStatesFor(askerGot(), 1)
		if len(states) != 1 || states[0].Open != nil || states[0].Close != nil {
			t.Fatalf("問い直した画面への返事 = %+v, want 空の返事を 1 通", states)
		}
		if got := otherGot(); len(got) != 0 {
			t.Fatalf("ほかの画面に %d 通届いた", len(got))
		}
	})

	t.Run("知らないセッション: 何も送らない", func(t *testing.T) {
		s := newTestServer()
		asker, askerGot := registerCaptureUI(s)
		s.handleApprovalResync(asker, 9)
		if got := askerGot(); len(got) != 0 {
			t.Fatalf("知らないセッションへの問い直しで %d 通届いた", len(got))
		}
	})

	t.Run("接続直後のまとめの最中: 返事はまとめを追い越さず後ろに積む", func(t *testing.T) {
		s := newTestServer()
		registerTestSession(s, 1, "codex")
		asker, askerGot := registerCaptureUI(s)
		s.sessionsMu.Lock()
		asker.priming = true
		s.sessionsMu.Unlock()
		s.handleApprovalResync(asker, 1)
		if got := askerGot(); len(got) != 0 {
			t.Fatalf("まとめの最中に %d 通を直接送った", len(got))
		}
		s.sessionsMu.Lock()
		queued := len(asker.queued)
		s.sessionsMu.Unlock()
		if queued != 1 {
			t.Fatalf("積んだ返事 = %d 通, want 1", queued)
		}
	})
}

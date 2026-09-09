package hub

import (
	"encoding/json"
	"testing"

	"many-ai-cli/internal/proto"
)

// These fixtures intentionally contain only semantic prompt fragments. They
// model the #5 failure without importing session logs, paths, or provider
// secrets into the regression suite.
func TestApprovalCandidateKeyIgnoresReflowOnlyChanges(t *testing.T) {
	first := approvalCandidateKey("codex", "native", "Proceed with this change?", []proto.ApprovalOption{
		{Num: 1, Label: "Yes (Recommended)", SendText: "1\r"},
		{Num: 0, Label: "No", SendText: "0\r"},
	})
	reflow := approvalCandidateKey("codex", "native", "  Proceed   with this\nchange?  ", []proto.ApprovalOption{
		{Num: 1, Label: "Yes", SendText: "1\r"},
		{Num: 0, Label: "No — status changed", SendText: "0\r"},
	})
	if first == "" || first != reflow {
		t.Fatalf("reflow-only changes must share candidate key: first=%q reflow=%q", first, reflow)
	}

	newQuestion := approvalCandidateKey("codex", "native", "Proceed with the other change?", []proto.ApprovalOption{
		{Num: 1, Label: "Yes", SendText: "1\r"},
		{Num: 0, Label: "No", SendText: "0\r"},
	})
	if first == newQuestion {
		t.Fatalf("different questions must not share candidate key: %q", first)
	}
}

func TestApprovalCandidateKeyDistinguishesNativeAndMarker(t *testing.T) {
	native := approvalCandidateKey("codex", "native", "Proceed?", []proto.ApprovalOption{{Num: 1, SendText: "1\r"}})
	marker := approvalMarkerCandidateKey("codex", "[MANY-AI-CLI]\nQ1 Proceed?\n1. Yes\n[/MANY-AI-CLI]")
	if native == marker {
		t.Fatalf("native and marker candidates must retain separate provenance: %q", native)
	}

	markerReflow := approvalMarkerCandidateKey("codex", "[MANY-AI-CLI]\nQ1   Proceed?\n1. Yes (Recommended)\n[/MANY-AI-CLI]")
	if marker != markerReflow {
		t.Fatalf("marker label/whitespace reflow must share candidate key: %q vs %q", marker, markerReflow)
	}
}

func TestGenericApprovalCandidateIncludesCommandSubject(t *testing.T) {
	options := []proto.ApprovalOption{{Num: 1, Label: "Allow"}, {Num: 0, Label: "Reject"}}
	status := approvalCandidateKeyWithContext("claude", "native", "Do you want to proceed?", "Bash command\ngit status", options)
	destructive := approvalCandidateKeyWithContext("claude", "native", "Do you want to proceed?", "Bash command\nrm -rf ./dist", options)
	if status == destructive {
		t.Fatalf("generic approval questions with different commands must not collide: %q", status)
	}

	reflow := approvalCandidateKeyWithContext("claude", "native", "Do you want to proceed?", "Bash command\n  git   status  ", options)
	if status != reflow {
		t.Fatalf("command subject reflow must retain candidate key: %q vs %q", status, reflow)
	}
}

func TestApprovalReplayWireFieldsAreOptionalAndRoundTrip(t *testing.T) {
	legacy, err := json.Marshal(proto.Message{Type: "pty_data", SessionID: 7, Data: []byte("old")})
	if err != nil {
		t.Fatal(err)
	}
	var decoded proto.Message
	if err := json.Unmarshal(legacy, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded.Replay || decoded.ReplayEpoch != 0 || decoded.ApprovalSourceEpoch != 0 {
		t.Fatalf("legacy message must keep replay fields at zero: %+v", decoded)
	}

	want := proto.Message{
		Type:                   "pty_data",
		SessionID:              7,
		Data:                   []byte("replayed"),
		Replay:                 true,
		ReplayEpoch:            3,
		ApprovalSourceEpoch:    5,
		ApprovalCandidateKey:   "candidate123456",
		ApprovalCandidateShape: "codex\nnative\nProceed?\n1:1\\r",
		ApprovalConsumed:       true,
		ApprovalConsumedEpoch:  4,
	}
	wire, err := json.Marshal(want)
	if err != nil {
		t.Fatal(err)
	}
	var got proto.Message
	if err := json.Unmarshal(wire, &got); err != nil {
		t.Fatal(err)
	}
	if !got.Replay || got.ReplayEpoch != want.ReplayEpoch ||
		got.ApprovalSourceEpoch != want.ApprovalSourceEpoch ||
		got.ApprovalCandidateKey != want.ApprovalCandidateKey ||
		got.ApprovalCandidateShape != want.ApprovalCandidateShape ||
		!got.ApprovalConsumed || got.ApprovalConsumedEpoch != want.ApprovalConsumedEpoch {
		t.Fatalf("replay metadata did not round trip: got=%+v want=%+v", got, want)
	}
}

func TestApprovalCandidateEpochSuppressesAnsweredReplayButAllowsNewPrompt(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 11, "codex")
	approval := func(label, context string) *nativeApproval {
		a := &nativeApproval{
			Kind:     "native",
			Question: "Proceed with this change?",
			Context:  context,
			Options: []proto.ApprovalOption{
				{Num: 1, Label: label, SendText: "1\r"},
				{Num: 0, Label: "No", SendText: "0\r"},
			},
		}
		a.Sig = approvalCandidateKey("codex", a.Kind, a.Question, a.Options)
		return a
	}

	first := approval("Yes (Recommended)", "status line A")
	s.handleNativeApprovalDetection(11, first)
	candidate := approvalCandidateKey("codex", first.Kind, first.Question, first.Options)
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            11,
		ApprovalSig:          first.Sig,
		ApprovalCandidateKey: candidate,
		ApprovalSourceEpoch:  1,
	})

	// The same logical prompt with a reflowed label/context is replayed. It must
	// remain suppressed in the consumed epoch.
	s.handleNativeApprovalDetection(11, approval("Yes", "status line B"))
	s.sessionsMu.Lock()
	if ses.nativeApprovalSig != "" {
		t.Fatalf("answered replay restored native approval: %+v", ses)
	}
	markApprovalUserTurnBoundaryLocked(ses)
	s.sessionsMu.Unlock()

	// A live prompt boundary permits the same question to be asked again.
	s.handleNativeApprovalDetection(11, approval("Yes", "new prompt"))
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.nativeApprovalSig == "" || ses.nativeApprovalSourceEpoch != 2 {
		t.Fatalf("new prompt was not allowed in a new epoch: sig=%q epoch=%d", ses.nativeApprovalSig, ses.nativeApprovalSourceEpoch)
	}
}

func TestDelayedApprovalConsumedDoesNotSuppressNewEpoch(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 13, "codex")
	approval := &nativeApproval{
		Kind:     "native",
		Question: "Proceed with this change?",
		Options: []proto.ApprovalOption{
			{Num: 1, Label: "Yes", SendText: "1\r"},
			{Num: 0, Label: "No", SendText: "0\r"},
		},
	}
	approval.Sig = approvalCandidateKey("codex", approval.Kind, approval.Question, approval.Options)
	candidate := approval.Sig

	// Model the answer arriving after the confirmed input already opened the
	// next live generation, while the old candidate is still the active Hub
	// observation.
	s.sessionsMu.Lock()
	ses.approvalSourceEpoch = 2
	ses.nativeApprovalSig = approval.Sig
	ses.nativeApprovalCandidateKey = candidate
	ses.nativeApprovalSourceEpoch = 1
	s.sessionsMu.Unlock()
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            13,
		ApprovalSig:          approval.Sig,
		ApprovalCandidateKey: candidate,
		ApprovalSourceEpoch:  1,
	})

	s.sessionsMu.Lock()
	if ses.approvalEpochPending || ses.approvalConsumedEpoch != 1 {
		s.sessionsMu.Unlock()
		t.Fatalf("delayed answer must remain in its old epoch: pending=%v consumed_epoch=%d", ses.approvalEpochPending, ses.approvalConsumedEpoch)
	}
	s.sessionsMu.Unlock()
	s.handleNativeApprovalDetection(13, approval)
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.nativeApprovalSourceEpoch != 2 || ses.nativeApprovalCandidateKey != candidate {
		t.Fatalf("same candidate must be available in the new epoch: key=%q epoch=%d", ses.nativeApprovalCandidateKey, ses.nativeApprovalSourceEpoch)
	}
}

func TestApprovalMarkerCandidateReflowAndEpoch(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 12, "codex")
	first := extractApprovalMarkerBlock([]string{
		"[MANY-AI-CLI]",
		"Q1 proceed with this change?",
		"1. Yes (Recommended)",
		"2. No",
		"[/MANY-AI-CLI]",
	})
	if first == nil || !s.maybeBroadcastApprovalMarker(12, first, ses.lastOutputAt) {
		t.Fatal("first marker should be accepted")
	}
	reflow := extractApprovalMarkerBlock([]string{
		"[MANY-AI-CLI]",
		"Q1   proceed   with this change?",
		"1. Yes",
		"2. No — repainted",
		"[/MANY-AI-CLI]",
	})
	if reflow == nil || s.maybeBroadcastApprovalMarker(12, reflow, ses.lastOutputAt) {
		t.Fatal("marker reflow must not be broadcast twice")
	}
	candidate := ses.approvalMarkerCandidateKey
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            12,
		ApprovalSig:          first.Sig,
		ApprovalCandidateKey: candidate,
		ApprovalSourceEpoch:  1,
	})
	s.sessionsMu.Lock()
	markApprovalUserTurnBoundaryLocked(ses)
	s.sessionsMu.Unlock()
	newPrompt := extractApprovalMarkerBlock([]string{
		"[MANY-AI-CLI]",
		"Q1 proceed with this change?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	})
	if newPrompt == nil || !s.maybeBroadcastApprovalMarker(12, newPrompt, ses.lastOutputAt) {
		t.Fatal("same question must be available again after a new prompt boundary")
	}
}

func TestApprovalBoundaryCarriesConsumedMarkerAcrossEpoch(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 14, "codex")
	ses.vt = newVTBuffer(80, 8)
	ses.vt.scrollback = []string{
		"[MANY-AI-CLI]",
		"Q1 proceed with this change?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	}
	marker := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil || !s.maybeBroadcastApprovalMarker(14, marker, ses.lastOutputAt) {
		t.Fatal("marker should be accepted before consumption")
	}
	candidate := ses.approvalMarkerCandidateKey
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            14,
		ApprovalSig:          marker.Sig,
		ApprovalCandidateKey: candidate,
		ApprovalSourceEpoch:  1,
	})

	s.sessionsMu.Lock()
	markApprovalUserTurnBoundaryLocked(ses)
	if ses.approvalSourceEpoch != 2 || !ses.approvalEpochPending || ses.approvalConsumedEpoch != 2 {
		s.sessionsMu.Unlock()
		t.Fatalf("consumed marker was not carried to the next epoch: epoch=%d pending=%v consumed_epoch=%d", ses.approvalSourceEpoch, ses.approvalEpochPending, ses.approvalConsumedEpoch)
	}
	s.sessionsMu.Unlock()

	replay := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if replay == nil || s.maybeBroadcastApprovalMarker(14, replay, ses.lastOutputAt) {
		t.Fatal("the consumed marker in scrollback must remain suppressed after the boundary")
	}
}

// 回答済み記録の持ち越しは、その回答済みブロックが VT から抽出できる間ずっと続く。
// 回数では区切らない（2026-08-27 にユーザー判断で撤廃。それまでは 1 回だけで、
// 答えた後にユーザーが 2 回何かを送っただけで、画面に出たままの回答済みブロックが
// 未回答の候補へ戻っていた。bugfix_approval-panel-reappears-live-after-answer_2026-08-27.md）。
func TestApprovalBoundaryCarriesConsumedMarkerWhileVisible(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 21, "codex")
	ses.vt = newVTBuffer(80, 8)
	ses.vt.scrollback = []string{
		"[MANY-AI-CLI]",
		"Q1 proceed with this change?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	}
	marker := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil || !s.maybeBroadcastApprovalMarker(21, marker, ses.lastOutputAt) {
		t.Fatal("marker should be accepted before consumption")
	}
	candidate := ses.approvalMarkerCandidateKey
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            21,
		ApprovalSig:          marker.Sig,
		ApprovalCandidateKey: candidate,
		ApprovalSourceEpoch:  1,
	})

	// 回答済みブロックが画面に出ている限り、ユーザーターンが何回来ても持ち越す。
	for turn := 2; turn <= 4; turn++ {
		s.sessionsMu.Lock()
		markApprovalUserTurnBoundaryLocked(ses)
		epoch := ses.approvalSourceEpoch
		consumedEpoch := ses.approvalConsumedEpoch
		pending := ses.approvalEpochPending
		s.sessionsMu.Unlock()
		if epoch != uint64(turn) {
			t.Fatalf("turn %d: boundary should advance the epoch: epoch=%d", turn, epoch)
		}
		if consumedEpoch != uint64(turn) || !pending {
			t.Fatalf("turn %d: carry was not extended: consumed_epoch=%d pending=%v", turn, consumedEpoch, pending)
		}
		replay := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
		if replay == nil || s.maybeBroadcastApprovalMarker(21, replay, ses.lastOutputAt) {
			t.Fatalf("turn %d: the answered block must stay suppressed while it is still on screen", turn)
		}
	}
}

// 持ち越しの解除条件は回数ではなく画面。エージェントが別のものを描いた時点で
// 持ち越しは止まり、そのあとに同じ質問が出れば新しい候補として配信される。
func TestApprovalBoundaryStopsCarryWhenBlockLeavesScreen(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 22, "codex")
	ses.vt = newVTBuffer(80, 8)
	block := []string{
		"[MANY-AI-CLI]",
		"Q1 proceed with this change?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	}
	ses.vt.scrollback = append([]string(nil), block...)
	marker := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil || !s.maybeBroadcastApprovalMarker(22, marker, ses.lastOutputAt) {
		t.Fatal("marker should be accepted before consumption")
	}
	s.markNativeApprovalConsumed(proto.Message{
		SessionID:            22,
		ApprovalSig:          marker.Sig,
		ApprovalCandidateKey: ses.approvalMarkerCandidateKey,
		ApprovalSourceEpoch:  1,
	})

	// 回答済みブロックが画面から消える（エージェントが次の作業を描いた）。
	ses.vt.scrollback = []string{"agent moved on", "no marker on screen"}
	s.sessionsMu.Lock()
	markApprovalUserTurnBoundaryLocked(ses)
	consumedEpoch := ses.approvalConsumedEpoch
	epoch := ses.approvalSourceEpoch
	pending := ses.approvalEpochPending
	s.sessionsMu.Unlock()
	if consumedEpoch == epoch || pending {
		t.Fatalf("carry must stop once the block is gone: epoch=%d consumed_epoch=%d pending=%v", epoch, consumedEpoch, pending)
	}

	// 同じ質問を出し直したら、新しい候補として出る（永久抑止になっていない）。
	ses.vt.scrollback = append([]string(nil), block...)
	reask := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if reask == nil || !s.maybeBroadcastApprovalMarker(22, reask, ses.lastOutputAt) {
		t.Fatal("a re-ask after the block left the screen must surface again")
	}
}

// F-12 must not depend on one literal question string: any fixed wording that
// leaves the command in the prompt body has to fold the command into identity.
func TestGenericApprovalSubjectIsNotPhraseSpecific(t *testing.T) {
	options := []proto.ApprovalOption{{Num: 1}, {Num: 2}}
	for _, question := range []string{
		"Do you want to proceed?",
		"Allow this command?",
		"Continue?",
	} {
		safe := approvalCandidateKeyWithContext("claude", "native", question, "Bash command\ngit status", options)
		danger := approvalCandidateKeyWithContext("claude", "native", question, "Bash command\nrm -rf ./dist", options)
		if safe == danger {
			t.Fatalf("question %q: two different commands collided on one candidate key", question)
		}
	}
	// A question that already names the command is left alone.
	withCmd := approvalIdentityQuestionWithContext("Run: git status", "Bash command\nrm -rf ./dist", options)
	if withCmd != "Run: git status" {
		t.Fatalf("question already naming a command must not absorb the body: %q", withCmd)
	}
}

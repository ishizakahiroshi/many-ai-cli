package hub

import "testing"

func TestExtractApprovalMarkerBlockMultiline(t *testing.T) {
	lines := []string{
		"thinking...",
		"[MANY-AI-CLI]",
		"Q1 scope?",
		" 1. Model only (Recommended)",
		" 2. All selectors",
		" N. User specifies",
		"[/MANY-AI-CLI]",
	}

	got := extractApprovalMarkerBlock(lines)
	if got == nil {
		t.Fatal("extractApprovalMarkerBlock returned nil")
	}
	want := "[MANY-AI-CLI]\nQ1 scope?\n 1. Model only (Recommended)\n 2. All selectors\n N. User specifies\n[/MANY-AI-CLI]"
	if got.Block != want {
		t.Fatalf("block = %q, want %q", got.Block, want)
	}
	if got.Sig == "" {
		t.Fatal("sig is empty")
	}
}

func TestExtractApprovalMarkerBlockLastCompleteBlock(t *testing.T) {
	lines := []string{
		"[MANY-AI-CLI]",
		"first?",
		"1. Yes",
		"[/MANY-AI-CLI]",
		"noise",
		"[MANY-AI-CLI]",
		"second?",
		"1. Yes",
		"2. No",
		"[/MANY-AI-CLI]",
	}

	got := extractApprovalMarkerBlock(lines)
	if got == nil {
		t.Fatal("extractApprovalMarkerBlock returned nil")
	}
	if got.Block != "[MANY-AI-CLI]\nsecond?\n1. Yes\n2. No\n[/MANY-AI-CLI]" {
		t.Fatalf("block = %q", got.Block)
	}
}

func TestExtractApprovalMarkerBlockIgnoresIncomplete(t *testing.T) {
	lines := []string{
		"[MANY-AI-CLI]",
		"Q1 missing close?",
		"1. Yes",
	}

	if got := extractApprovalMarkerBlock(lines); got != nil {
		t.Fatalf("extractApprovalMarkerBlock = %+v, want nil", got)
	}
}

func TestMaybeBroadcastApprovalMarkerDedupesSameBlock(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	marker := extractApprovalMarkerBlock([]string{
		"[MANY-AI-CLI]",
		"Q1 proceed?",
		"1. Yes",
		"[/MANY-AI-CLI]",
	})
	if marker == nil {
		t.Fatal("marker nil")
	}

	if !s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("first marker should be accepted")
	}
	if ses.approvalMarkerSig != marker.Sig {
		t.Fatalf("approvalMarkerSig = %q, want %q", ses.approvalMarkerSig, marker.Sig)
	}
	if s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("same marker should be deduped")
	}
}

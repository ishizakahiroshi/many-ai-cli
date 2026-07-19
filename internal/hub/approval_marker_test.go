package hub

import (
	"strings"
	"testing"
)

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

// Grok 差分再描画で色 CSI だけが揺れた同一質問は、同一 sig として dedupe する。
func TestApprovalMarkerSignatureIgnoresAnsiFlicker(t *testing.T) {
	plain := "[MANY-AI-CLI]\nスターター取込の本線？\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	// 実測 s22: 同一 clean 本文でも \x1b[48;2;... の余白/色だけが変わる
	coloredA := "[MANY-AI-CLI]\n\x1b[39m\x1b[48;2;1;1;1mスターター取込の本線？\x1b[K\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	coloredB := "[MANY-AI-CLI]\n\x1b[39m\x1b[48;2;20;20;20mスターター取込の本線？\x1b[K\n1. A (Recommended)\n2. B\nN. User specifies\n[/MANY-AI-CLI]"
	sigPlain := approvalMarkerSignature(plain)
	sigA := approvalMarkerSignature(coloredA)
	sigB := approvalMarkerSignature(coloredB)
	if sigPlain == "" || sigA == "" {
		t.Fatal("empty signature")
	}
	if sigPlain != sigA || sigA != sigB {
		t.Fatalf("ANSI flicker must share sig: plain=%q a=%q b=%q", sigPlain, sigA, sigB)
	}

	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	first := &approvalMarkerBlock{Block: coloredA, Sig: sigA}
	second := &approvalMarkerBlock{Block: coloredB, Sig: sigB}
	if !s.maybeBroadcastApprovalMarker(1, first, ses.lastOutputAt) {
		t.Fatal("first colored marker should broadcast")
	}
	if s.maybeBroadcastApprovalMarker(1, second, ses.lastOutputAt) {
		t.Fatal("ANSI-only variant must be deduped (no rebroadcast)")
	}
}

func TestApprovalMarkerSignatureDiffersForDifferentQuestions(t *testing.T) {
	a := approvalMarkerSignature("[MANY-AI-CLI]\nQ1 first?\n1. Yes\n[/MANY-AI-CLI]")
	b := approvalMarkerSignature("[MANY-AI-CLI]\nQ1 second?\n1. Yes\n[/MANY-AI-CLI]")
	if a == b {
		t.Fatalf("different questions must not share sig: %q", a)
	}
}

func TestExtractApprovalMarkerBlockSpansScrollback(t *testing.T) {
	// 画面高 4 行の VT に、開始〜閉じが画面をまたぐ長いマーカーブロックを流す。
	// TailLines（現在画面のみ）では不完全、TailLinesWithScrollback では完全ブロックが取れること。
	vt := newVTBuffer(80, 4)
	blockLines := []string{
		"[MANY-AI-CLI]",
		"Q1 first long question that should scroll out of the viewport?",
		" 1. Option A for Q1 (Recommended)",
		" 2. Option B for Q1",
		" N. User specifies",
		"Q2 second question also long enough?",
		" 3. Option C for Q2 (Recommended)",
		" 4. Option D for Q2",
		" N. User specifies",
		"[/MANY-AI-CLI]",
	}
	for _, line := range blockLines {
		vt.Write([]byte(line + "\r\n"))
	}
	// 現在画面だけでは完全ブロックが取れないこと（scrollback 経路の必要性を必須 assert）
	screenOnly := extractApprovalMarkerBlock(vt.TailLines(vtTailLinesForApproval))
	if screenOnly != nil && strings.Contains(screenOnly.Block, "Q1 first") && strings.Contains(screenOnly.Block, "Q2 second") {
		t.Fatalf("screen-only extract unexpectedly got full block (scrollback path not required): %q", screenOnly.Block)
	}
	// 画面 4 行 + 最終空行で、開始が scrollback 側に落ちている想定
	withSB := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if withSB == nil {
		t.Fatalf("scrollback extract returned nil; screen=%v sb=%v lines=%v", vt.Lines(), vt.scrollback, vt.TailLinesWithScrollback(50))
	}
	if !strings.Contains(withSB.Block, "[MANY-AI-CLI]") || !strings.Contains(withSB.Block, "[/MANY-AI-CLI]") {
		t.Fatalf("incomplete block: %q", withSB.Block)
	}
	if !strings.Contains(withSB.Block, "Q1 first") || !strings.Contains(withSB.Block, "Q2 second") {
		t.Fatalf("missing questions in block: %q", withSB.Block)
	}
}

// clear 後も scrollback に古い完全ブロックが残るが、同一 sig は再 broadcast されないこと
// （敵対レビュー P1: ゴースト承認の抑止）。
func TestMaybeBroadcastApprovalMarkerNoGhostAfterScreenClear(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	if ses.vt == nil {
		ses.vt = newVTBuffer(80, 4)
	}
	vt := ses.vt
	blockLines := []string{
		"[MANY-AI-CLI]",
		"Q1 ghost candidate after clear?",
		" 1. Yes (Recommended)",
		" 2. No",
		"[/MANY-AI-CLI]",
	}
	for _, line := range blockLines {
		vt.Write([]byte(line + "\r\n"))
	}
	// 押し出して scrollback に載せる
	for i := 0; i < 6; i++ {
		vt.Write([]byte("noise line that scrolls\r\n"))
	}
	marker := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil {
		t.Fatalf("expected marker in scrollback; sb=%v screen=%v", vt.scrollback, vt.Lines())
	}
	if !s.maybeBroadcastApprovalMarker(1, marker, ses.lastOutputAt) {
		t.Fatal("first broadcast should succeed")
	}
	// 画面クリアしても scrollback は残る（仕様）
	vt.Write([]byte("\x1b[2J\x1b[H"))
	if len(vt.scrollback) == 0 {
		t.Fatal("scrollback should survive clear")
	}
	again := extractApprovalMarkerBlock(vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if again == nil {
		t.Fatal("old complete block still extractable from scrollback after clear")
	}
	if again.Sig != marker.Sig {
		t.Fatalf("sig mismatch after clear: %q vs %q", again.Sig, marker.Sig)
	}
	if s.maybeBroadcastApprovalMarker(1, again, ses.lastOutputAt) {
		t.Fatal("same sig after clear must be deduped (no ghost approval_marker)")
	}
}

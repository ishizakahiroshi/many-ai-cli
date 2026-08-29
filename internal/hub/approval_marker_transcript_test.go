package hub

import (
	"strings"
	"testing"
	"time"
)

// transcriptFooter は承認マーカーブロックの後ろに続く後書き。
// 実測（2026-08-29 セッション 16）では応答フッター 14 行 + CLI のチローム 7 行 =
// 終了マーカーの下 21 行で、openlessCloseBottomWindow（16 行）を超えていた。
var transcriptFooter = []string{
	"", "---", "",
	"実行ファイル:", "なし", "",
	"変更ファイル:", "なし", "",
	"使用モデル: テスト", "",
	"次にすること:", "確認する", "",
	"✻ Brewed for 4m 10s · done 13:54",
}

var transcriptMarkerBody = []string{
	approvalMarkerOpen,
	"  前置きの 1 行目。ここは画面高を超えると描かれない。",
	"  前置きの 2 行目。",
	"",
	"  Q1 この方針で進めますか?",
	"  1. [進める] このまま進める (Recommended)",
	"  2. [やめる] いったん保留にする",
	"   N. User specifies",
	approvalMarkerClose,
}

// overflowedInkScreen は「開始マーカーが端末へ 1 バイトも届かず、終了マーカーの下に
// 後書きが 21 行続く」画面を組み立てる。Ink の塗り直しではあふれた行が newLine() を
// 通らないので scrollback にも残らない。
func overflowedInkScreen() *vtBuffer {
	rows := []string{
		"  本文の見えている部分 1",
		"  本文の見えている部分 2",
		"  Q1 この方針で進めますか?",
		"  1. [進める] このまま進める (Recommended)",
		"  2. [やめる] いったん保留にする",
		"   N. User specifies",
		approvalMarkerClose,
	}
	rows = append(rows, transcriptFooter...)
	rows = append(rows, inkChrome...)
	vt := newVTBuffer(60, len(rows))
	writeInkFrame(vt, rows)
	return vt
}

func transcriptAssistantText() string {
	return strings.Join(append(append([]string(nil), transcriptMarkerBody...), transcriptFooter...), "\n")
}

// TestTranscriptSuppliesMarkerWhenScreenDroppedIt は本 bugfix の中身そのもの。
// VT ミラーからは何も取り出せない画面でも、CLI のトランスクリプトから同じ質問が
// 完全な形で取り出せ、承認候補として記録されることを固定する。
func TestTranscriptSuppliesMarkerWhenScreenDroppedIt(t *testing.T) {
	vt := overflowedInkScreen()
	if marker := extractApprovalMarkerBlockFromVT(vt); marker != nil {
		t.Fatalf("VT からブロックが取れてしまった: %q（この画面では取れないのが前提）", marker.Block)
	}

	marker := approvalMarkerFromTranscriptText(transcriptAssistantText())
	if marker == nil {
		t.Fatal("approvalMarkerFromTranscriptText = nil, want ブロック")
	}
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		t.Fatalf("classify = %q, want ok: %q", reason, marker.Block)
	}
	for _, want := range []string{"前置きの 1 行目", "Q1 この方針で進めますか?", "1. [進める]", "N. User specifies"} {
		if !strings.Contains(marker.Block, want) {
			t.Fatalf("ブロックに %q が無い: %q", want, marker.Block)
		}
	}

	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.vt = vt
	ses.agentChatPath = "transcript.jsonl"
	s.scanTranscriptApprovalMarkers(1, []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
	}, false, time.Now())

	if ses.approvalMarkerCandidateKey == "" {
		t.Fatal("承認候補が立っていない（トランスクリプト経路で配信されていない）")
	}
	if ses.approvalMarkerSig != marker.Sig {
		t.Fatalf("approvalMarkerSig = %q, want %q", ses.approvalMarkerSig, marker.Sig)
	}
}

// 供給元は 1 セッションにつき 1 つ。トランスクリプトが読めているセッションでは
// VT からマーカーを立てない（両方から立てると candidateKey が 2 通りになる）。
func TestReplayApprovalSkipsVTMarkerWhenTranscriptIsSource(t *testing.T) {
	block := strings.Join(transcriptMarkerBody, "\n")
	newSession := func(path string) (*Server, *session) {
		s := newTestServer()
		ses := registerTestSession(s, 1, "claude")
		ses.vt = newVTBuffer(60, 20)
		ses.vt.Write([]byte(strings.ReplaceAll(block, "\n", "\r\n")))
		ses.agentChatPath = path
		return s, ses
	}

	s, ses := newSession("transcript.jsonl")
	s.evaluateReplayApproval(1)
	if ses.approvalMarkerCandidateKey != "" {
		t.Fatal("トランスクリプトが供給元なのに VT からマーカーを立てている")
	}

	s, ses = newSession("")
	s.evaluateReplayApproval(1)
	if ses.approvalMarkerCandidateKey == "" {
		t.Fatal("トランスクリプトが未解決のセッションでは VT が供給元でなければならない")
	}
}

func TestApprovalMarkerSourceIsTranscriptOnlyWhenReadable(t *testing.T) {
	cases := []struct {
		provider string
		path     string
		want     bool
	}{
		{"claude", "transcript.jsonl", true},
		{"codex", "rollout.jsonl", true},
		{"claude", "", false},
		{"grok", "transcript.jsonl", false},
		{"copilot", "transcript.jsonl", false},
	}
	for _, tc := range cases {
		ses := &session{Provider: tc.provider, agentChatPath: tc.path}
		if got := approvalMarkerSourceIsTranscriptLocked(ses); got != tc.want {
			t.Fatalf("provider=%q path=%q: got %v, want %v", tc.provider, tc.path, got, tc.want)
		}
	}
	if approvalMarkerSourceIsTranscriptLocked(nil) {
		t.Fatal("nil session が true を返した")
	}
}

// prime（reattach でパーサ状態を作り直した 1 回目）は、すでに書かれたレコードから
// 表示を組み直すだけの baseline。古い世代の質問まで承認として立て直さないこと。
// 最後の assistant メッセージだけは「まだ止まっている質問」なので配信する。
func TestTranscriptPrimeOnlyRestoresTheLastMessage(t *testing.T) {
	older := []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
		{Role: "user", Kind: "text", Text: "1"},
		{Role: "assistant", Kind: "text", Text: "了解しました。進めます。"},
	}
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.agentChatPath = "transcript.jsonl"
	s.scanTranscriptApprovalMarkers(1, older, true, time.Now())
	if ses.approvalMarkerCandidateKey != "" {
		t.Fatal("prime が過去ターンの質問を承認として立て直した")
	}

	pending := append(append([]agentChatMessage(nil), older...),
		agentChatMessage{Role: "assistant", Kind: "text", Text: transcriptAssistantText()})
	s.scanTranscriptApprovalMarkers(1, pending, true, time.Now())
	if ses.approvalMarkerCandidateKey == "" {
		t.Fatal("prime が「まだ止まっている質問」を復元していない")
	}
}

// トランスクリプトが読めていないセッション（他 provider・パス未解決）へは
// この経路から一切配信しない。
func TestTranscriptScanIgnoresNonTranscriptSessions(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "grok")
	ses.agentChatPath = "transcript.jsonl"
	s.scanTranscriptApprovalMarkers(1, []agentChatMessage{
		{Role: "assistant", Kind: "text", Text: transcriptAssistantText()},
	}, false, time.Now())
	if ses.approvalMarkerCandidateKey != "" {
		t.Fatal("grok セッションへトランスクリプト経路から配信した")
	}
}

func TestPTYChunkClosesApprovalMarker(t *testing.T) {
	if !ptyChunkClosesApprovalMarker([]byte("\x1b[12;1H   " + approvalMarkerClose + "\r\n")) {
		t.Fatal("終了マーカーを含むチャンクを検出できていない")
	}
	if ptyChunkClosesApprovalMarker([]byte("  Q1 進めますか?\r\n  1. はい\r\n")) {
		t.Fatal("終了マーカーの無いチャンクで前倒しを掛けている")
	}
}

// 前倒しは「まだ発火していないタイマー」を組み直すときだけ行う。発火済みの
// タイマーを組み直すと同じ generation の poll が二重に走り、両者が同じ
// agentChatParseState（可変 map）を触る。
func TestKickAgentChatPollDoesNotRescheduleFiredTimer(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")

	// 走っていないセッションは何もしない。
	s.sessionsMu.Lock()
	s.kickAgentChatPollLocked(1, ses)
	s.sessionsMu.Unlock()
	if ses.agentChatTimer != nil {
		t.Fatal("停止中のセッションでタイマーを作った")
	}

	fired := make(chan struct{}, 1)
	ses.agentChatRunning = true
	ses.agentChatTimer = time.AfterFunc(time.Millisecond, func() { fired <- struct{}{} })
	select {
	case <-fired:
	case <-time.After(2 * time.Second):
		t.Fatal("テスト用タイマーが発火しなかった")
	}
	before := ses.agentChatTimer
	s.sessionsMu.Lock()
	s.kickAgentChatPollLocked(1, ses)
	s.sessionsMu.Unlock()
	if ses.agentChatTimer != before {
		t.Fatal("発火済みタイマーを組み直した（poll が二重に走る）")
	}

	ses.agentChatTimer = time.AfterFunc(time.Hour, func() {})
	pending := ses.agentChatTimer
	s.sessionsMu.Lock()
	s.kickAgentChatPollLocked(1, ses)
	s.sessionsMu.Unlock()
	if ses.agentChatTimer == pending {
		t.Fatal("未発火タイマーが前倒しされていない")
	}
	ses.agentChatTimer.Stop()
}

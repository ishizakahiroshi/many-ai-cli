package sessionlog

import (
	"strings"
	"testing"
)

func TestWriteTranscript(t *testing.T) {
	in := strings.NewReader(strings.Join([]string{
		`{"type":"session_start","ts":"2026-05-11T19:34:57+09:00","session_id":7,"provider":"codex","cwd":"C:\\dev\\many-ai-cli","pid":19552}`,
		`{"type":"pty_output","ts":"2026-05-11T19:34:58+09:00","session_id":7,"text":"\u001b]0;title\u0007Hello\r\n"}`,
		`{"type":"user_input","ts":"2026-05-11T19:35:08+09:00","session_id":7,"text":"現在時刻を教えて\r"}`,
		`{"type":"session_end","ts":"2026-05-11T19:36:00+09:00","session_id":7,"state":"completed","exit_code":0}`,
		"",
	}, "\n"))
	var out strings.Builder
	if err := WriteTranscript(in, &out); err != nil {
		t.Fatal(err)
	}
	got := out.String()
	for _, want := range []string{
		"session_start #7 codex",
		"[output]\nHello",
		"user_input\n> 現在時刻を教えて",
		"session_end state=completed exit_code=0",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("transcript missing %q in:\n%s", want, got)
		}
	}
	if strings.Contains(got, "\x1b") {
		t.Fatalf("transcript still contains escape bytes: %q", got)
	}
}

func TestIsThinkingNoiseLine(t *testing.T) {
	noise := []string{
		"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)",
		"thinking with medium effort✳3thinking with medium effort",
		"Imp·rmpovri✶osviisng✶i...n*g...",
		"1.8924  Opus 4.8  ↑111.0k ↓764",
		"auto mode on (shift+tab to cycle)",
		"⠋⠙ working...", // braille スピナーが 2 個以上並ぶ進捗フレーム
	}
	for _, s := range noise {
		if !IsThinkingNoiseLine(s) {
			t.Errorf("IsThinkingNoiseLine(%q) = false, want true", s)
		}
	}
	keep := []string{
		"I am thinking about the architecture here.", // "thinking" だがスピナーグリフ無し
		"Done. Updated 3 files ✓",                     // ✓ は対象外の dingbat
		"● Read(internal/hub/server.go)",              // ツール呼び出し行
		"Here is the summary of the changes:",
		"  - item one",
	}
	for _, s := range keep {
		if IsThinkingNoiseLine(s) {
			t.Errorf("IsThinkingNoiseLine(%q) = true, want false", s)
		}
	}
}

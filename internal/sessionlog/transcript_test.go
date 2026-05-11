package sessionlog

import (
	"strings"
	"testing"
)

func TestWriteTranscript(t *testing.T) {
	in := strings.NewReader(strings.Join([]string{
		`{"type":"session_start","ts":"2026-05-11T19:34:57+09:00","session_id":7,"provider":"codex","cwd":"C:\\dev\\ai-cli-hub","pid":19552}`,
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

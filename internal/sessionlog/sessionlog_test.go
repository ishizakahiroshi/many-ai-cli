package sessionlog

import (
	"strings"
	"testing"
	"time"
)

func TestBaseName(t *testing.T) {
	meta := Metadata{
		SessionID: 12,
		Provider:  "claude",
		CWD:       `C:\dev\ai-cli-hub`,
		StartedAt: time.Date(2026, 5, 10, 9, 15, 32, 0, time.Local),
	}
	got := BaseName(meta)
	if !strings.HasPrefix(got, "claude_2026-05-10_091532_ai-cli-hub_s12") {
		t.Fatalf("unexpected basename: %s", got)
	}
}

func TestSanitizeFilePart(t *testing.T) {
	if got := SanitizeFilePart(`foo:bar<>baz`); got != "foo_bar__baz" {
		t.Fatalf("sanitize failed: %s", got)
	}
	if got := SanitizeFilePart(""); got != "no-project" {
		t.Fatalf("empty sanitize failed: %s", got)
	}
	long := strings.Repeat("a", 120)
	if got := SanitizeFilePart(long); len(got) != 80 {
		t.Fatalf("expected 80 chars, got %d", len(got))
	}
}

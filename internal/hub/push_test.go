package hub

import (
	"strings"
	"testing"
	"unicode/utf8"
)

func TestApprovalPushURLDoesNotIncludeToken(t *testing.T) {
	got := approvalPushURL(42)
	if got != "/?session_id=42" {
		t.Fatalf("approvalPushURL = %q, want session-only URL", got)
	}
	if strings.Contains(got, "token=") {
		t.Fatalf("approvalPushURL must not include Hub token: %q", got)
	}
}

func TestTruncateUTF8BytesKeepsValidUTF8(t *testing.T) {
	input := strings.Repeat("承認", 100)
	got := truncateUTF8Bytes(input, pushPayloadMaxLen)
	if len(got) > pushPayloadMaxLen {
		t.Fatalf("truncated body length = %d, want <= %d", len(got), pushPayloadMaxLen)
	}
	if !utf8.ValidString(got) {
		t.Fatalf("truncated body is not valid UTF-8: %q", got)
	}
}

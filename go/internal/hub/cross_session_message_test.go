package hub

import "testing"

func TestDetectCrossSessionMessage(t *testing.T) {
	got := detectCrossSessionMessage([]string{
		"normal output",
		"  ⏺ Message from researcher  ",
	})
	if got == nil {
		t.Fatal("detectCrossSessionMessage returned nil")
	}
	if got.Sender != "researcher" {
		t.Fatalf("sender = %q, want researcher", got.Sender)
	}
	if got.Line != "⏺ Message from researcher" {
		t.Fatalf("line = %q, want normalized header", got.Line)
	}
	if got.Signature == "" {
		t.Fatal("signature is empty")
	}
}

func TestDetectCrossSessionMessageIgnoresOrdinaryOutput(t *testing.T) {
	for _, lines := range [][]string{
		{"message received from a tool"},
		{"Message from"},
		{"received message to researcher"},
		{"the assistant said Message from researcher"},
	} {
		if got := detectCrossSessionMessage(lines); got != nil {
			t.Fatalf("detectCrossSessionMessage(%q) = %#v, want nil", lines, got)
		}
	}
}

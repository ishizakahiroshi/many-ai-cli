package hub

import (
	"testing"

	"many-ai-cli/internal/proto"
)

func TestAutoApprovalInputUsesNumberedPositiveOption(t *testing.T) {
	got := autoApprovalInput([]proto.ApprovalOption{
		{Num: 1, Label: "Allow once"},
		{Num: 2, Label: "Reject"},
	})
	if got != "1\r" {
		t.Fatalf("autoApprovalInput = %q, want numbered allow input", got)
	}
}

func TestAutoApprovalInputDoesNotGuessUnnumberedPositiveOption(t *testing.T) {
	if got := autoApprovalInput([]proto.ApprovalOption{{Label: "Allow once"}}); got != "" {
		t.Fatalf("autoApprovalInput = %q, want empty without a sendable option", got)
	}
}

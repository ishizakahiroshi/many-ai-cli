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

func TestApprovalOptionInputRejectsNegativeAndPersistentLabels(t *testing.T) {
	negative := []string{"Don't run", "Do not allow", "No", "Deny once", "Cancel (esc)"}
	for _, label := range negative {
		t.Run(label, func(t *testing.T) {
			option := proto.ApprovalOption{Label: label, SendText: "n", Num: 2}
			if got := autoApprovalInput([]proto.ApprovalOption{option}); got != "" {
				t.Fatalf("autoApprovalInput(%q) = %q, want empty", label, got)
			}
			if got := oneTapRejectInput([]proto.ApprovalOption{option}); got == "" {
				t.Fatalf("oneTapRejectInput(%q) = empty, want explicit rejection input", label)
			}
		})
	}

	persistent := []string{"Yes, and don't ask again for this command", "Allow all similar for this session", "Always allow"}
	for _, label := range persistent {
		t.Run(label, func(t *testing.T) {
			if got := autoApprovalInput([]proto.ApprovalOption{{Label: label, SendText: "y", Num: 1}}); got != "" {
				t.Fatalf("autoApprovalInput(%q) = %q, want empty", label, got)
			}
		})
	}
}

func TestApprovalOptionInputAcceptsProviderFixturePositiveLabels(t *testing.T) {
	labels := []string{"Yes, allow once", "Run (once) (y)", "Allow once (y)", "Yes, proceed"}
	for _, label := range labels {
		t.Run(label, func(t *testing.T) {
			if got := autoApprovalInput([]proto.ApprovalOption{{Label: label, SendText: "y", Num: 1}}); got != "y" {
				t.Fatalf("autoApprovalInput(%q) = %q, want y", label, got)
			}
		})
	}
}

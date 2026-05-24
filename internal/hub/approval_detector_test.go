package hub

import "testing"

func TestDetectNativeApprovalClaude(t *testing.T) {
	lines := []string{
		"",
		"Allow tool: Bash",
		"",
		"❯ 1. Yes, allow once",
		"  2. Yes, allow for this session",
		"  3. No",
	}
	got := detectNativeApproval("claude", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 3 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	if !got.Options[0].IsCurrent || got.Options[0].Label != "Yes, allow once" {
		t.Fatalf("option 1 = %+v", got.Options[0])
	}
	if got.Sig == "" {
		t.Fatal("sig is empty")
	}
}

func TestDetectNativeApprovalCodexShortcut(t *testing.T) {
	lines := []string{
		"Command requires approval",
		"Run: rm -rf ./tmp",
		"",
		"❯ Yes (y)",
		"  Yes, and don't ask again for this command (p)",
		"  No (n)",
		"  Cancel (esc)",
	}
	got := detectNativeApproval("codex", lines)
	if got == nil {
		t.Fatal("detectNativeApproval returned nil")
	}
	if got.Kind != "native_codex_shortcut" {
		t.Fatalf("kind = %q", got.Kind)
	}
	if len(got.Options) != 4 {
		t.Fatalf("options len = %d", len(got.Options))
	}
	if got.Options[0].SendText != "y" {
		t.Fatalf("option 1 send_text = %q", got.Options[0].SendText)
	}
	if got.Options[3].SendText != "\x1b" {
		t.Fatalf("option 4 send_text = %q", got.Options[3].SendText)
	}
}

func TestDetectNativeApprovalFalsePositiveNumberedList(t *testing.T) {
	lines := []string{
		"Implementation plan:",
		"❯ 1. Read the files",
		"  2. Edit the code",
		"  3. Run tests",
	}
	if got := detectNativeApproval("claude", lines); got != nil {
		t.Fatalf("detectNativeApproval = %+v, want nil", got)
	}
}

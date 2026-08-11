package wrapper

import (
	"regexp"
	"testing"
)

func TestPrepareClaudeSessionArgsInjectsFreshID(t *testing.T) {
	args, id, err := prepareClaudeSessionArgs("claude", []string{"--verbose"})
	if err != nil {
		t.Fatal(err)
	}
	if id == "" || len(args) < 2 || args[0] != "--session-id" || args[1] != id {
		t.Fatalf("unexpected args/id: %#v %q", args, id)
	}
	if !regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`).MatchString(id) {
		t.Fatalf("not an RFC 4122 UUIDv4: %q", id)
	}
}

func TestPrepareClaudeSessionArgsDoesNotOverrideExistingSession(t *testing.T) {
	tests := [][]string{
		{"--session-id", "existing"},
		{"--session-id=existing"},
		{"--resume", "existing"},
		{"--resume=existing"},
		{"-r", "existing"},
		{"--continue"},
		{"--fork-session"},
	}
	for _, input := range tests {
		args, id, err := prepareClaudeSessionArgs("claude", input)
		if err != nil {
			t.Fatal(err)
		}
		if id != "" || len(args) != len(input) {
			t.Errorf("args %v were changed to %v with id %q", input, args, id)
		}
	}
}

func TestPrepareClaudeSessionArgsOnlyTargetsClaude(t *testing.T) {
	for _, provider := range []string{"codex", "grok", "copilot"} {
		args, id, err := prepareClaudeSessionArgs(provider, []string{"--verbose"})
		if err != nil {
			t.Fatal(err)
		}
		if id != "" || len(args) != 1 || args[0] != "--verbose" {
			t.Errorf("provider %s unexpectedly changed args: %v id=%q", provider, args, id)
		}
	}
}

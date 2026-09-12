package wrapper

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/headless"
)

// The prompt travels as a file so it never reaches an argument list or a
// process table, and the file is gone once it has been read.
func TestTakePromptFileReadsAndDeletes(t *testing.T) {
	path := filepath.Join(t.TempDir(), "prompt-test.md")
	if err := os.WriteFile(path, []byte("review the diff\n"), 0o600); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	got, err := takePromptFile(path)
	if err != nil {
		t.Fatalf("takePromptFile: %v", err)
	}
	if got != "review the diff\n" {
		t.Errorf("prompt = %q, want the file's contents", got)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Errorf("the prompt file is still there (stat err = %v)", err)
	}

	// No prompt file at all is valid: a headless run with no instruction gets
	// an immediate EOF rather than an error.
	if got, err := takePromptFile(""); err != nil || got != "" {
		t.Errorf("takePromptFile(\"\") = %q, %v, want empty and nil", got, err)
	}
	// A missing file is an error, not an empty prompt: silently running a
	// worker with no instruction would burn a turn for nothing.
	if _, err := takePromptFile(filepath.Join(t.TempDir(), "absent.md")); err == nil {
		t.Error("a missing prompt file must fail")
	}
}

// Raw provider output is written only where the user opted into raw session
// logs. The gate is the same one the raw PTY log uses, for the same reason:
// the bytes carry keys and tokens unmasked.
func TestHeadlessRawLogPrefixFollowsTheSessionLogOptIn(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.LogDir = filepath.Join(t.TempDir(), "logs")
	when := time.Date(2026, 9, 12, 10, 30, 0, 0, time.UTC)

	if prefix := headlessRawLogPrefix(cfg, "claude", when); prefix != "" {
		t.Errorf("prefix = %q, want no raw log while log.session_enabled is off", prefix)
	}
	cfg.Log.SessionEnabled = true
	prefix := headlessRawLogPrefix(cfg, "claude", when)
	if !strings.Contains(filepath.ToSlash(prefix), "/headless/claude_20260912-103000") {
		t.Errorf("prefix = %q, want the headless log directory and a timestamped name", prefix)
	}
	if headlessRawLogPrefix(nil, "claude", when) != "" {
		t.Error("a nil config must not produce a log path")
	}
}

// The last line of a headless session says how the process ended, because that
// — not anything the model wrote — is the completion signal (元設計 11 節).
func TestHeadlessEndLine(t *testing.T) {
	cases := []struct {
		name   string
		result headless.Result
		want   string
	}{
		{"completed", headless.Result{State: "completed"}, "[result] exit=0"},
		{"failed", headless.Result{State: "error", ExitCode: 3}, "[result] exit=3"},
		{"timeout", headless.Result{State: "error", TimedOut: true}, "[error] run timed out"},
		{"stopped", headless.Result{State: "error", Canceled: true}, "[error] run was stopped"},
	}
	for _, tc := range cases {
		if got := headlessEndLine(tc.result); got != tc.want {
			t.Errorf("%s: headlessEndLine = %q, want %q", tc.name, got, tc.want)
		}
	}
}

// A provider with no definition cannot run headless, and says so instead of
// starting something else (親 plan D2). This is the wrapper-side backstop: the
// Hub already refuses the same request, but `many-ai-cli wrap codex --headless`
// typed by hand reaches here first.
//
// codex is the example because it is the one built-in provider deliberately left
// out of the table (`codex exec` rejects the approval flag every codex child is
// given — the reason is written out in internal/config/headless.go). If it ever
// gains a row, pick another provider without one rather than deleting the test.
func TestRunHeadlessSessionRefusesAProviderWithoutADefinition(t *testing.T) {
	err := runHeadlessSession(headlessSession{cfg: &config.Config{}, logger: quietLogger(), provider: "codex"})
	if err == nil {
		t.Fatal("a provider with no headless definition must fail")
	}
	if !strings.Contains(err.Error(), "headless") {
		t.Errorf("err = %v, want it to name the missing headless support", err)
	}
}

// The two flag combinations that cannot mean anything are rejected before the
// wrapper does anything at all (no Hub, no process).
func TestRunRejectsImpossibleHeadlessFlagCombinations(t *testing.T) {
	cfg := &config.Config{}
	err := Run(cfg, quietLogger(), "claude", []string{"--headless", "--subscription-login"})
	if err == nil || !strings.Contains(err.Error(), "subscription-login") {
		t.Fatalf("err = %v, want the headless/login combination rejected", err)
	}
	err = Run(cfg, quietLogger(), "claude", []string{"--prompt-file", "somewhere.md"})
	if err == nil || !strings.Contains(err.Error(), "prompt-file") {
		t.Fatalf("err = %v, want --prompt-file without --headless rejected", err)
	}
}

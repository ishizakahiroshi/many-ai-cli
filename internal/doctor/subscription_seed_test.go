package doctor

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/subscription"
)

func writeSeedFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

// A profile created before seeding existed keeps working, so nothing fails.
// The only way its missing rules and skills become visible is this row.
func TestSubscriptionSeedCheckReportsMissingEntries(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")
	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# rules\n")
	writeSeedFile(t, filepath.Join(home, ".claude", "settings.json"), `{}`)

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	if err := os.MkdirAll(profileDir, 0o700); err != nil {
		t.Fatal(err)
	}

	check := subscriptionSeedCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	})
	if check == nil {
		t.Fatal("a profile missing the user's rules and settings produced no row")
	}
	if check.Level != Warn {
		t.Errorf("level = %v, want %v", check.Level, Warn)
	}
	if !strings.Contains(check.Message, "CLAUDE.md") {
		t.Errorf("message does not name the missing entry: %q", check.Message)
	}
	if check.Fix == "" {
		t.Error("a warning with no fix leaves the user with nowhere to go")
	}
}

// An unused feature must not add lines to the report.
func TestSubscriptionSeedCheckStaysQuietWhenNothingIsMissing(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	if err := os.MkdirAll(profileDir, 0o700); err != nil {
		t.Fatal(err)
	}
	// No default configuration exists, so there is nothing to carry.
	if check := subscriptionSeedCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	}); check != nil {
		t.Fatalf("reported drift with no default configuration: %+v", check)
	}

	// A profile that has not been signed into yet is already reported by the
	// row above; repeating it here would only double the noise.
	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# rules\n")
	if check := subscriptionSeedCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: false,
	}); check != nil {
		t.Fatalf("reported drift for a profile with no directory: %+v", check)
	}
}

// OpenCode loses nothing when a profile is selected, so it must never grow a
// drift row telling the user to fix something that is not broken.
func TestSubscriptionSeedCheckIgnoresProvidersWithNothingToCarry(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	profileDir := filepath.Join(t.TempDir(), "opencode", "work")
	if err := os.MkdirAll(profileDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if check := subscriptionSeedCheck("opencode", "opencode / work", subscription.Entry{
		Provider: "opencode", ID: "work", ProfileDir: profileDir, Exists: true,
	}); check != nil {
		t.Fatalf("opencode reported drift: %+v", check)
	}
}

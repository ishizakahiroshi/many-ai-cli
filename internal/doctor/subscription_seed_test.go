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

// A profile's plain copy of CLAUDE.md silently stops tracking the default the
// moment the user turns their own default into a symlink (e.g. syncing it from
// another real path). Nothing else notices this: the file is still there, it
// still has content, it is just frozen. This is exactly the scenario recorded
// in the plan's "作者環境で起きたこと" section.
func TestSubscriptionRuleFileCheckWarnsWhenDefaultLinkedButProfileCopied(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	realTarget := filepath.Join(t.TempDir(), "real-claude-md", "CLAUDE.md")
	writeSeedFile(t, realTarget, "# the one true rules file\n")
	claudeDir := filepath.Join(home, ".claude")
	if err := os.MkdirAll(claudeDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(realTarget, filepath.Join(claudeDir, "CLAUDE.md")); err != nil {
		t.Skipf("cannot create the default-side symlink this test needs: %v", err)
	}

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	writeSeedFile(t, filepath.Join(profileDir, "CLAUDE.md"), "# stale copy from before the default became a link\n")

	check := subscriptionRuleFileCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	})
	if check == nil {
		t.Fatal("a plain copy sitting under a linked default produced no row")
	}
	if check.Level != Warn {
		t.Errorf("level = %v, want %v", check.Level, Warn)
	}
	if !strings.Contains(check.Message, "リンク") {
		t.Errorf("message does not mention the link mismatch: %q", check.Message)
	}
	if check.Fix == "" {
		t.Error("a warning with no fix leaves the user with nowhere to go")
	}
}

// Two plain files that used to match can drift apart with nothing to notice:
// the profile's copy is never touched again after seeding.
func TestSubscriptionRuleFileCheckWarnsWhenCopyDrifted(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# rules, edited after seeding\n")
	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	writeSeedFile(t, filepath.Join(profileDir, "CLAUDE.md"), "# rules, as they were at seed time\n")

	check := subscriptionRuleFileCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	})
	if check == nil {
		t.Fatal("two files with different content produced no row")
	}
	if check.Level != Warn {
		t.Errorf("level = %v, want %v", check.Level, Warn)
	}
	if !strings.Contains(check.Message, "内容が違います") {
		t.Errorf("message does not say the content differs: %q", check.Message)
	}
	if check.Fix == "" {
		t.Error("a warning with no fix leaves the user with nowhere to go")
	}
}

// Nothing to report: a profile that already mirrors the default as a symlink
// cannot drift, and byte-identical plain copies have not drifted (yet).
func TestSubscriptionRuleFileCheckStaysQuietWhenLinkedOrIdentical(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# whatever the default currently says\n")

	// (a) the profile's CLAUDE.md is itself a symlink, so it always reads
	// whatever the default currently says and can never drift.
	linkedProfile := filepath.Join(t.TempDir(), "claude", "linked")
	if err := os.MkdirAll(linkedProfile, 0o700); err != nil {
		t.Fatal(err)
	}
	linkTarget := filepath.Join(t.TempDir(), "profile-owned-rules.md")
	writeSeedFile(t, linkTarget, "# profile owns this\n")
	if err := os.Symlink(linkTarget, filepath.Join(linkedProfile, "CLAUDE.md")); err != nil {
		t.Skipf("cannot create a symlink in this environment: %v", err)
	}
	if check := subscriptionRuleFileCheck("claude", "claude / linked", subscription.Entry{
		Provider: "claude", ID: "linked", ProfileDir: linkedProfile, Exists: true,
	}); check != nil {
		t.Fatalf("a profile whose rule file is itself a symlink cannot drift, got %+v", check)
	}

	// (b) both sides are plain files with byte-identical content.
	identicalProfile := filepath.Join(t.TempDir(), "claude", "identical")
	writeSeedFile(t, filepath.Join(identicalProfile, "CLAUDE.md"), "# whatever the default currently says\n")
	if check := subscriptionRuleFileCheck("claude", "claude / identical", subscription.Entry{
		Provider: "claude", ID: "identical", ProfileDir: identicalProfile, Exists: true,
	}); check != nil {
		t.Fatalf("byte-identical copies must not be reported as drifted, got %+v", check)
	}

	// (c) not seeded yet at all — that gap is subscriptionSeedCheck's job, not
	// this one's.
	notSeededProfile := filepath.Join(t.TempDir(), "claude", "not-seeded")
	if err := os.MkdirAll(notSeededProfile, 0o700); err != nil {
		t.Fatal(err)
	}
	if check := subscriptionRuleFileCheck("claude", "claude / not-seeded", subscription.Entry{
		Provider: "claude", ID: "not-seeded", ProfileDir: notSeededProfile, Exists: true,
	}); check != nil {
		t.Fatalf("a profile with no rule file yet must stay quiet here, got %+v", check)
	}
}

// マーカー文字列は internal/wrapper/approval_rules.go / delegation_inject.go の
// 同名の unexported 定数と同じ値を手で揃えたもの（package をまたぐため import
// できない）。値がずれると「注入ブロックを模したはずが実際は無視されない」形で
// このテストが偽陰性化するので、wrapper 側の定数を変えたらここも合わせる。
const (
	testSharedBlockStart = "<!-- many-ai-cli:approval-rules -->"
	testSharedBlockEnd   = "<!-- /many-ai-cli:approval-rules -->"
	testDelegationStart  = "<!-- many-ai-cli:delegation -->"
	testDelegationEnd    = "<!-- /many-ai-cli:delegation -->"
)

// セッションが動いている間、profile 側の AGENTS.md/CLAUDE.md にだけ承認ルールの
// 共有ブロックが注入される（既定側には入らない）。sha256File がブロックを除かずに
// 比べると、この間ずっと「内容が違います」が偽陽性で出る（C5 の背景）。
func TestSubscriptionRuleFileCheckIgnoresInjectedApprovalBlock(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	body := "# rules shared by default and profile\n"
	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), body)

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	profileBody := body + "\n" + testSharedBlockStart + "\ninjected approval rules\n" + testSharedBlockEnd + "\n"
	writeSeedFile(t, filepath.Join(profileDir, "CLAUDE.md"), profileBody)

	if check := subscriptionRuleFileCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	}); check != nil {
		t.Fatalf("an injected approval-rules block was treated as drift: %+v", check)
	}
}

// delegation ブロックも同じ経路で codex/copilot/cursor-agent/opencode の
// AGENTS.md へ注入される（delegation_inject.go）。こちらも除いてから比べる。
func TestSubscriptionRuleFileCheckIgnoresInjectedDelegationBlock(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	body := "# rules shared by default and profile\n"
	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), body)

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	profileBody := body + "\n" + testDelegationStart + "\ninjected delegation guidance\n" + testDelegationEnd + "\n"
	writeSeedFile(t, filepath.Join(profileDir, "CLAUDE.md"), profileBody)

	if check := subscriptionRuleFileCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	}); check != nil {
		t.Fatalf("an injected delegation block was treated as drift: %+v", check)
	}
}

// 陽性対照: ブロック除去が本文側まで飲み込んでいないかを固定する。共有ブロックは
// 付いているが、それとは別にブロック外の本文を 1 文字変えてあるので、除去後も
// 「内容が違います」が出なければならない。
func TestSubscriptionRuleFileCheckWarnsWhenBodyDiffersOutsideInjectedBlock(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(subscription.ClaudeConfigDirEnv, "")

	writeSeedFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# rules, original\n")

	profileDir := filepath.Join(t.TempDir(), "claude", "work")
	// "original" -> "originaL"（末尾 1 文字だけ変更）。
	profileBody := "# rules, originaL\n\n" + testSharedBlockStart + "\ninjected approval rules\n" + testSharedBlockEnd + "\n"
	writeSeedFile(t, filepath.Join(profileDir, "CLAUDE.md"), profileBody)

	check := subscriptionRuleFileCheck("claude", "claude / work", subscription.Entry{
		Provider: "claude", ID: "work", ProfileDir: profileDir, Exists: true,
	})
	if check == nil {
		t.Fatal("a one-character body difference outside the injected block produced no row (block stripping over-matched)")
	}
	if !strings.Contains(check.Message, "内容が違います") {
		t.Errorf("message does not say the content differs: %q", check.Message)
	}
}

package subscription

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"testing"
)

// isolateVendorHome points every vendor CLI's default location at an empty
// temporary home, so no test in this package can read — or seed from — the
// machine's real ~/.claude, ~/.codex or ~/.grok.
func isolateVendorHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv(ClaudeConfigDirEnv, "")
	t.Setenv(CodexHomeEnv, "")
	t.Setenv(GrokHomeEnv, "")
	return home
}

func writeFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func appliedContains(result SeedResult, dest string) bool {
	return slices.Contains(result.Applied, dest)
}

func TestSeedCarriesClaudeSettingsAndSkills(t *testing.T) {
	home := isolateVendorHome(t)
	claudeDir := filepath.Join(home, ".claude")
	writeFile(t, filepath.Join(claudeDir, "CLAUDE.md"), "# common rules\n")
	writeFile(t, filepath.Join(claudeDir, "settings.json"), `{"permissions":{"allow":["Bash(ls)"]}}`)
	writeFile(t, filepath.Join(claudeDir, "skills", "demo", "SKILL.md"), "# demo skill\n")

	profile := filepath.Join(t.TempDir(), "claude", "work")
	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}

	for _, dest := range []string{"CLAUDE.md", "settings.json", "skills"} {
		if !appliedContains(result, dest) {
			t.Errorf("%q was not seeded; applied=%v failed=%v", dest, result.Applied, result.Failed)
		}
	}
	rules, err := os.ReadFile(filepath.Join(profile, "CLAUDE.md"))
	if err != nil || string(rules) != "# common rules\n" {
		t.Fatalf("CLAUDE.md = %q, %v", rules, err)
	}
	// The skills directory is linked, so a skill added later must be visible
	// through the profile with no second seeding pass.
	writeFile(t, filepath.Join(claudeDir, "skills", "added-later", "SKILL.md"), "# later\n")
	if _, err := os.Stat(filepath.Join(profile, "skills", "added-later", "SKILL.md")); err != nil {
		t.Fatalf("a skill added after seeding is not visible through the profile: %v", err)
	}
}

// The credential is the one thing profiles exist to keep apart. No adapter may
// list it, whatever else it decides to carry.
func TestSeedNeverCarriesCredentials(t *testing.T) {
	home := isolateVendorHome(t)
	forbidden := map[string][]string{
		"claude": {".credentials.json"},
		"codex":  {"auth.json"},
		"grok":   {"auth.json", "auth.json.lock"},
	}
	for provider, names := range forbidden {
		for _, entry := range seedEntriesFor(provider) {
			for _, name := range names {
				if filepath.Base(entry.Source) == name || entry.Dest == name {
					t.Errorf("%s adapter seeds the credential file %q", provider, name)
				}
			}
			// Every source must sit inside the vendor's own default directory.
			if rel, err := filepath.Rel(home, entry.Source); err != nil || rel == ".." {
				t.Errorf("%s adapter seeds from outside the home directory: %s", provider, entry.Source)
			}
		}
	}
}

// Seeding must never overwrite what a profile already holds: a value the user
// changed inside a profile is the one they meant.
func TestSeedIsAdditiveOnly(t *testing.T) {
	home := isolateVendorHome(t)
	claudeDir := filepath.Join(home, ".claude")
	writeFile(t, filepath.Join(claudeDir, "settings.json"), `{"theme":"default"}`)
	writeFile(t, filepath.Join(claudeDir, "CLAUDE.md"), "# from default\n")

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"), `{"theme":"chosen-inside-the-profile"}`)

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if appliedContains(result, "settings.json") {
		t.Error("an existing settings.json was overwritten")
	}
	kept, err := os.ReadFile(filepath.Join(profile, "settings.json"))
	if err != nil || string(kept) != `{"theme":"chosen-inside-the-profile"}` {
		t.Fatalf("settings.json = %q, %v", kept, err)
	}
	if !appliedContains(result, "CLAUDE.md") {
		t.Error("a missing entry must still be carried in alongside an existing one")
	}

	// A second pass has nothing left to do.
	again, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("second EnsureProfileDir: %v", err)
	}
	if again.Any() {
		t.Errorf("second pass changed something: applied=%v failed=%v", again.Applied, again.Failed)
	}
}

// .claude.json holds account identity next to a couple of real preferences.
// Only the named keys may cross; carrying the file whole would give a profile
// the default account's identity.
func TestSeedClaudeStateCopiesOnlyNamedKeys(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{}`)
	writeFile(t, filepath.Join(home, ".claude.json"), `{
	  "oauthAccount": {"emailAddress": "someone@example.com"},
	  "userID": "abc123",
	  "machineID": "def456",
	  "claudeInChromeDefaultEnabled": true,
	  "hasCompletedClaudeInChromeOnboarding": true,
	  "numStartups": 3075
	}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	if _, err := EnsureProfileDir("claude", profile); err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(profile, ".claude.json"))
	if err != nil {
		t.Fatalf("read seeded .claude.json: %v", err)
	}
	var got map[string]json.RawMessage
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("seeded .claude.json is not valid JSON: %v", err)
	}
	for _, key := range claudeCarriedStateKeys {
		if _, ok := got[key]; !ok {
			t.Errorf("%q was not carried into the profile", key)
		}
	}
	for _, key := range []string{"oauthAccount", "userID", "machineID", "numStartups"} {
		if _, ok := got[key]; ok {
			t.Errorf("%q must not be carried into a profile", key)
		}
	}
}

// A profile pointed at by CLAUDE_CONFIG_DIR is another profile, not a default.
// Seeding from it would give a new account the previous account's state.
func TestSeedIgnoresDefaultsInsideTheSubscriptionsTree(t *testing.T) {
	home := isolateVendorHome(t)
	configDir := filepath.Join(home, ".many-ai-cli")
	other := filepath.Join(configDir, "subscriptions", "claude", "other")
	writeFile(t, filepath.Join(other, "CLAUDE.md"), "# another profile\n")
	t.Setenv(ClaudeConfigDirEnv, other)

	// vendorDefaultDir must fall back to ~/.claude, which does not exist here,
	// so nothing is pending.
	if entries := PendingSeedEntries("claude", filepath.Join(t.TempDir(), "new")); len(entries) != 0 {
		t.Fatalf("seeded from a profile directory: %#v", entries)
	}
}

// OpenCode keeps its config under XDG_CONFIG_HOME, which many-ai-cli never
// moves, so there is nothing for it to carry.
func TestOpenCodeHasNothingToSeed(t *testing.T) {
	isolateVendorHome(t)
	if entries := seedEntriesFor("opencode"); len(entries) != 0 {
		t.Fatalf("opencode declares seed entries: %#v", entries)
	}
	if entries := seedEntriesFor("copilot"); len(entries) != 0 {
		t.Fatalf("a provider with no adapter declares seed entries: %#v", entries)
	}
}

func TestSeedCarriesCodexAndGrokRules(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "AGENTS.md"), "# codex rules\n")
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "approval_policy = \"on-request\"\n")
	writeFile(t, filepath.Join(home, ".codex", "prompts", "P.md"), "# prompt\n")
	writeFile(t, filepath.Join(home, ".grok", "AGENTS.md"), "# grok rules\n")
	writeFile(t, filepath.Join(home, ".grok", "trusted_folders.toml"), "[folders]\n")

	root := t.TempDir()
	codex, err := EnsureProfileDir("codex", filepath.Join(root, "codex", "work"))
	if err != nil {
		t.Fatalf("EnsureProfileDir(codex): %v", err)
	}
	for _, dest := range []string{"AGENTS.md", "config.toml", "prompts"} {
		if !appliedContains(codex, dest) {
			t.Errorf("codex did not seed %q; applied=%v failed=%v", dest, codex.Applied, codex.Failed)
		}
	}
	grok, err := EnsureProfileDir("grok", filepath.Join(root, "grok", "work"))
	if err != nil {
		t.Fatalf("EnsureProfileDir(grok): %v", err)
	}
	for _, dest := range []string{"AGENTS.md", "trusted_folders.toml"} {
		if !appliedContains(grok, dest) {
			t.Errorf("grok did not seed %q; applied=%v failed=%v", dest, grok.Applied, grok.Failed)
		}
	}
	// config.toml is absent from the default home, so it must simply be skipped.
	if appliedContains(grok, "config.toml") {
		t.Error("grok seeded a config.toml that does not exist in the default home")
	}
}

// The link must survive on Windows without Developer Mode, where os.Symlink
// fails and the junction fallback has to take over.
func TestLinkDirWorksWithoutSymlinkPrivilege(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "target")
	writeFile(t, filepath.Join(target, "file.txt"), "content\n")
	link := filepath.Join(root, "link")

	if err := linkDir(target, link); err != nil {
		if runtime.GOOS == "windows" {
			t.Fatalf("linkDir failed on Windows even with the junction fallback: %v", err)
		}
		t.Fatalf("linkDir: %v", err)
	}
	body, err := os.ReadFile(filepath.Join(link, "file.txt"))
	if err != nil || string(body) != "content\n" {
		t.Fatalf("reading through the link = %q, %v", body, err)
	}
	// Lstat must see the link itself, which is how seeding decides "already
	// present" without following into the default configuration.
	if _, err := os.Lstat(link); err != nil {
		t.Fatalf("Lstat(link): %v", err)
	}
}

func TestPendingSeedEntriesReportsWhatIsMissing(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# rules\n")
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "CLAUDE.md"), "# already here\n")

	pending := PendingSeedEntries("claude", profile)
	var dests []string
	for _, entry := range pending {
		dests = append(dests, entry.Dest)
		if entry.Label == "" {
			t.Errorf("entry %q has no label for doctor to show", entry.Dest)
		}
	}
	if slices.Contains(dests, "CLAUDE.md") {
		t.Error("an entry the profile already has must not be reported as missing")
	}
	if !slices.Contains(dests, "settings.json") {
		t.Errorf("settings.json was not reported as missing; pending=%v", dests)
	}
	if got := PendingSeedEntries("claude", ""); got != nil {
		t.Errorf("an empty profile dir must report nothing, got %#v", got)
	}
}

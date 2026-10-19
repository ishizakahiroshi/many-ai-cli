package subscription

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"testing"

	"github.com/pelletier/go-toml/v2"

	"many-ai-cli/internal/config"
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

	// A profile that already has a symlink at CLAUDE.md (from a previous
	// SeedMirrorFile pass, or from the user replacing the copy by hand) must not
	// be touched either. PendingSeedEntries decides "already present" with
	// Lstat, which sees the link itself without following it.
	outsideTarget := filepath.Join(t.TempDir(), "profile-owned-rules.md")
	writeFile(t, outsideTarget, "# profile owns this\n")
	linkedProfile := filepath.Join(t.TempDir(), "claude", "linked")
	if err := os.MkdirAll(linkedProfile, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outsideTarget, filepath.Join(linkedProfile, "CLAUDE.md")); err != nil {
		t.Skipf("cannot create a symlink in this environment: %v", err)
	}
	linkedResult, err := EnsureProfileDir("claude", linkedProfile)
	if err != nil {
		t.Fatalf("EnsureProfileDir(linkedProfile): %v", err)
	}
	if appliedContains(linkedResult, "CLAUDE.md") {
		t.Error("an existing CLAUDE.md symlink was overwritten")
	}
	resolved, err := filepath.EvalSymlinks(filepath.Join(linkedProfile, "CLAUDE.md"))
	if err != nil {
		t.Fatalf("EvalSymlinks: %v", err)
	}
	wantTarget, err := filepath.EvalSymlinks(outsideTarget)
	if err != nil {
		t.Fatalf("EvalSymlinks(outsideTarget): %v", err)
	}
	if resolved != wantTarget {
		t.Errorf("CLAUDE.md symlink target = %q, want %q", resolved, wantTarget)
	}
}

func readProfileSettings(t *testing.T, profileDir string) map[string]json.RawMessage {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(profileDir, "settings.json"))
	if err != nil {
		t.Fatalf("read the profile's settings.json: %v", err)
	}
	var got map[string]json.RawMessage
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatalf("the profile's settings.json is not valid JSON: %v", err)
	}
	return got
}

// The default settings.json is the source of truth for the half of the file the
// user maintains by hand. A switch, a hook or a permission added there has to
// reach a profile that was seeded before it existed — the one-time copy is
// exactly how the user's own hooks stopped at the profile boundary with nothing
// on screen to say so.
func TestSeedSyncCarriesPolicyKeysIntoAnExistingProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"),
		`{"disableArtifact":true,"permissions":{"deny":["AskUserQuestion"]}}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"), `{"theme":"dark"}`)

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if appliedContains(result, "settings.json") {
		t.Error("an existing settings.json must be reported as synced, not carried in")
	}
	for _, key := range []string{"disableArtifact", "permissions"} {
		if !slices.Contains(result.Synced, key) {
			t.Errorf("%q was not reported as synced; synced=%v failed=%v", key, result.Synced, result.Failed)
		}
	}
	got := readProfileSettings(t, profile)
	if string(got["disableArtifact"]) != "true" {
		t.Errorf("disableArtifact = %s, want true", got["disableArtifact"])
	}
	if _, ok := got["permissions"]; !ok {
		t.Error("permissions never reached the profile")
	}
	// One stable shape — two-space indent, keys in sorted order — so that the
	// next pass compares against exactly what this one wrote and stops there.
	raw, err := os.ReadFile(filepath.Join(profile, "settings.json"))
	if err != nil {
		t.Fatal(err)
	}
	want := "{\n  \"disableArtifact\": true,\n  \"permissions\": {\n    \"deny\": [\n" +
		"      \"AskUserQuestion\"\n    ]\n  },\n  \"theme\": \"dark\"\n}\n"
	if string(raw) != want {
		t.Errorf("settings.json =\n%s\nwant\n%s", raw, want)
	}
}

// The keys Claude Code writes for itself belong to the profile. Taking them
// back from the default would undo the theme and the chosen model every time a
// session starts, which is the failure mode the state-key list exists to avoid.
func TestSeedSyncLeavesStateKeysToTheProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"),
		`{"theme":"light","modelSettings":{"model":"default"},"cleanupPeriodDays":30}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"),
		`{"theme":"dark","modelSettings":{"model":"chosen-inside-the-profile"}}`)

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	got := readProfileSettings(t, profile)
	if string(got["theme"]) != `"dark"` {
		t.Errorf("theme = %s, want the profile's own value", got["theme"])
	}
	var model struct {
		Model string `json:"model"`
	}
	if err := json.Unmarshal(got["modelSettings"], &model); err != nil {
		t.Fatalf("modelSettings is not an object: %v", err)
	}
	if model.Model != "chosen-inside-the-profile" {
		t.Errorf("modelSettings.model = %q, want the profile's own value", model.Model)
	}
	if string(got["cleanupPeriodDays"]) != "30" {
		t.Errorf("cleanupPeriodDays = %s: a policy key must still arrive", got["cleanupPeriodDays"])
	}
	for _, key := range []string{"theme", "modelSettings"} {
		if slices.Contains(result.Synced, key) {
			t.Errorf("%q belongs to the profile and must never be reported as synced", key)
		}
	}
}

// A key only the profile has is not something the sync can judge: it may be one
// the user set inside the profile, or one a newer CLI wrote. It stays.
func TestSeedSyncKeepsKeysOnlyTheProfileHas(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"cleanupPeriodDays":30}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"),
		`{"statusLine":{"type":"command","command":"profile-only"}}`)

	if _, err := EnsureProfileDir("claude", profile); err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	got := readProfileSettings(t, profile)
	if _, ok := got["statusLine"]; !ok {
		t.Error("a key only the profile had was dropped by the sync")
	}
	if string(got["cleanupPeriodDays"]) != "30" {
		t.Errorf("cleanupPeriodDays = %s: a policy key must still arrive", got["cleanupPeriodDays"])
	}
}

// A launch must not touch a profile that already agrees with the default, even
// when the two files are written differently. Comparing bytes rather than
// meaning would rewrite settings.json on every single session start.
func TestSeedSyncDoesNotRewriteAnAgreeingProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"disableArtifact":true}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	dest := filepath.Join(profile, "settings.json")
	// Four-space indent, the profile's own key first: the same meaning, other
	// bytes than this package would write.
	body := "{\n    \"theme\": \"dark\",\n    \"disableArtifact\": true\n}\n"
	writeFile(t, dest, body)
	before, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if len(result.Synced) != 0 {
		t.Errorf("nothing differs, yet the pass reported %v as synced", result.Synced)
	}
	after, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}
	if !after.ModTime().Equal(before.ModTime()) {
		t.Error("settings.json was rewritten even though nothing differs")
	}
	kept, err := os.ReadFile(dest)
	if err != nil || string(kept) != body {
		t.Fatalf("settings.json = %q, %v", kept, err)
	}
}

// A default configuration kept in a synced folder and linked into place is an
// ordinary setup. The sync has to read the file behind the link: reading the
// link itself would make the default look empty, and an empty default means
// emptying the profile to match.
func TestSeedSyncResolvesASymlinkedDefault(t *testing.T) {
	home := isolateVendorHome(t)
	claudeDir := filepath.Join(home, ".claude")
	realDefault := filepath.Join(t.TempDir(), "synced-config", "settings.json")
	writeFile(t, realDefault, `{"disableArtifact":true}`)
	if err := os.MkdirAll(claudeDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(realDefault, filepath.Join(claudeDir, "settings.json")); err != nil {
		t.Skipf("cannot create the default-side symlink this test needs: %v", err)
	}

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"), `{"theme":"dark"}`)

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if slices.Contains(result.Failed, "settings.json") {
		t.Fatalf("a symlinked default could not be read; failed=%v", result.Failed)
	}
	got := readProfileSettings(t, profile)
	if string(got["disableArtifact"]) != "true" {
		t.Errorf("disableArtifact = %s, want the value behind the symlink", got["disableArtifact"])
	}
}

// A default nobody can parse must leave the profile exactly as it was. Treating
// an unreadable default as an empty one would strip a working profile of the
// configuration it already had.
func TestSeedSyncLeavesTheProfileAloneWhenTheDefaultIsBroken(t *testing.T) {
	home := isolateVendorHome(t)
	// Truncated mid-object: the shape a half-written save leaves behind.
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"disableArtifact": true,`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	dest := filepath.Join(profile, "settings.json")
	body := `{"theme":"dark"}`
	writeFile(t, dest, body)

	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if !slices.Contains(result.Failed, "settings.json") {
		t.Errorf("an unparsable default must be reported as failed; failed=%v synced=%v",
			result.Failed, result.Synced)
	}
	kept, err := os.ReadFile(dest)
	if err != nil || string(kept) != body {
		t.Fatalf("settings.json = %q, %v", kept, err)
	}
}

// The sync changed a promise this tool had been making since profiles existed:
// what a profile already holds is never overwritten. A user who set a profile's
// settings.json apart on purpose has to be able to say so, or their profile is
// quietly rewritten at its next launch by a version they upgraded into.
func TestSeedSyncCanBeTurnedOffForOneProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"), `{"disableArtifact":true}`)

	off := false
	profile := filepath.Join(t.TempDir(), "claude", "work")
	dest := filepath.Join(profile, "settings.json")
	body := `{"theme":"dark"}`
	writeFile(t, dest, body)

	result, err := EnsureProfileDirFor("claude", profile,
		config.SubscriptionProfile{ID: "work", SettingsSync: &off})
	if err != nil {
		t.Fatalf("EnsureProfileDirFor: %v", err)
	}
	if len(result.Synced) != 0 {
		t.Errorf("a profile with settings_sync: false reported %v as synced", result.Synced)
	}
	if appliedContains(result, "settings.json") {
		t.Error("an existing settings.json must not be carried in again either")
	}
	kept, err := os.ReadFile(dest)
	if err != nil || string(kept) != body {
		t.Fatalf("settings.json = %q, %v; the default's keys must not arrive", kept, err)
	}

	// Turning the sync off returns the entry to the one-time copy, so a profile
	// with nothing there still gets the user's settings once. "Do not keep it in
	// step" is not "leave this profile at the CLI's factory state".
	fresh := filepath.Join(t.TempDir(), "claude", "fresh")
	freshResult, err := EnsureProfileDirFor("claude", fresh,
		config.SubscriptionProfile{ID: "fresh", SettingsSync: &off})
	if err != nil {
		t.Fatalf("EnsureProfileDirFor(fresh): %v", err)
	}
	if !appliedContains(freshResult, "settings.json") {
		t.Fatalf("a profile with no settings.json got none; applied=%v failed=%v",
			freshResult.Applied, freshResult.Failed)
	}
	if string(readProfileSettings(t, fresh)["disableArtifact"]) != "true" {
		t.Error("the first copy must still carry the user's settings whole")
	}
}

// enabledPlugins is written by the user and by `claude plugin enable` both, so
// which side owns it is a matter of how that person works rather than something
// this tool can decide for everyone. A profile that wants to keep its own set
// says so, and the sync then treats the key exactly like the ones the adapter
// already leaves alone.
func TestSeedSyncLeavesConfiguredProfileOwnedKeysAlone(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"),
		`{"enabledPlugins":{"demo@marketplace":true},"cleanupPeriodDays":30}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"),
		`{"enabledPlugins":{"demo@marketplace":false}}`)

	result, err := EnsureProfileDirFor("claude", profile, config.SubscriptionProfile{
		ID: "work", ProfileOwnedKeys: []string{"enabledPlugins"},
	})
	if err != nil {
		t.Fatalf("EnsureProfileDirFor: %v", err)
	}
	if slices.Contains(result.Synced, "enabledPlugins") {
		t.Errorf("a key the profile claimed must never be reported as synced; synced=%v", result.Synced)
	}
	got := readProfileSettings(t, profile)
	var plugins map[string]bool
	if err := json.Unmarshal(got["enabledPlugins"], &plugins); err != nil {
		t.Fatalf("enabledPlugins is not an object: %v", err)
	}
	if plugins["demo@marketplace"] {
		t.Error("the default's enabledPlugins overwrote the profile's own set")
	}
	if string(got["cleanupPeriodDays"]) != "30" {
		t.Errorf("cleanupPeriodDays = %s: every other policy key must still arrive", got["cleanupPeriodDays"])
	}

	// Without the setting the same pair is drift, so the profile's own value is
	// kept by the configuration and not by accident.
	plain := filepath.Join(t.TempDir(), "claude", "plain")
	writeFile(t, filepath.Join(plain, "settings.json"),
		`{"enabledPlugins":{"demo@marketplace":false}}`)
	plainResult, err := EnsureProfileDirFor("claude", plain, config.SubscriptionProfile{ID: "plain"})
	if err != nil {
		t.Fatalf("EnsureProfileDirFor(plain): %v", err)
	}
	if !slices.Contains(plainResult.Synced, "enabledPlugins") {
		t.Errorf("by default enabledPlugins is policy and comes from the default; synced=%v", plainResult.Synced)
	}
}

// The other direction: a user who keeps every profile on the same theme should
// not have to leave it to each profile just because the adapter lists it as one
// of the CLI's own keys.
func TestSeedSyncHandsDefaultWinsKeysBackToTheDefault(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "settings.json"),
		`{"theme":"light","modelSettings":{"model":"default"}}`)

	profile := filepath.Join(t.TempDir(), "claude", "work")
	writeFile(t, filepath.Join(profile, "settings.json"),
		`{"theme":"dark","modelSettings":{"model":"chosen-inside-the-profile"}}`)

	result, err := EnsureProfileDirFor("claude", profile, config.SubscriptionProfile{
		ID: "work", DefaultWinsKeys: []string{"theme"},
	})
	if err != nil {
		t.Fatalf("EnsureProfileDirFor: %v", err)
	}
	if !slices.Contains(result.Synced, "theme") {
		t.Errorf("theme was handed back to the default but not reported as synced; synced=%v", result.Synced)
	}
	got := readProfileSettings(t, profile)
	if string(got["theme"]) != `"light"` {
		t.Errorf("theme = %s, want the default's value", got["theme"])
	}
	// Only the named key moves. The rest of the adapter's list is untouched.
	var model struct {
		Model string `json:"model"`
	}
	if err := json.Unmarshal(got["modelSettings"], &model); err != nil {
		t.Fatalf("modelSettings is not an object: %v", err)
	}
	if model.Model != "chosen-inside-the-profile" {
		t.Errorf("modelSettings.model = %q, want the profile's own value", model.Model)
	}
}

func readProfileTOML(t *testing.T, profileDir, name string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(profileDir, name))
	if err != nil {
		t.Fatalf("read the profile's %s: %v", name, err)
	}
	var got map[string]any
	if err := toml.Unmarshal(data, &got); err != nil {
		t.Fatalf("the profile's %s is not valid TOML: %v", name, err)
	}
	return got
}

// Codex and Grok keep their settings in TOML, and the reason their profiles
// drift is the same one Claude's had: approval_policy, the MCP servers and the
// feature flags are maintained in one file and were copied out of it once.
func TestSeedSyncCarriesPolicyKeysIntoAnExistingCodexProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"),
		"approval_policy = \"on-request\"\n\n[mcp_servers.demo]\ncommand = \"demo-server\"\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"), "[tui]\nmodel_availability_nux = true\n")

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if appliedContains(result, "config.toml") {
		t.Error("an existing config.toml must be reported as synced, not carried in")
	}
	for _, key := range []string{"approval_policy", "mcp_servers"} {
		if !slices.Contains(result.Synced, key) {
			t.Errorf("%q was not reported as synced; synced=%v failed=%v", key, result.Synced, result.Failed)
		}
	}
	got := readProfileTOML(t, profile, "config.toml")
	if got["approval_policy"] != "on-request" {
		t.Errorf("approval_policy = %v, want the default's value", got["approval_policy"])
	}
	if _, ok := got["mcp_servers"]; !ok {
		t.Error("mcp_servers never reached the profile")
	}
	tui, ok := got["tui"].(map[string]any)
	if !ok || tui["model_availability_nux"] != true {
		t.Errorf("tui = %v: the profile's own table was lost in the merge", got["tui"])
	}
}

// The tables Codex writes for itself are the profile's. Taking them back would
// undo the chosen model and the dialogs the user already dismissed at every
// launch.
func TestSeedSyncLeavesCodexStateTablesToTheProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"),
		"model = \"the-default-model\"\nsandbox_mode = \"workspace-write\"\n\n[tui]\nmodel_availability_nux = false\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"),
		"model = \"the-model-this-profile-picked\"\n\n[tui]\nmodel_availability_nux = true\n")

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	got := readProfileTOML(t, profile, "config.toml")
	if got["model"] != "the-model-this-profile-picked" {
		t.Errorf("model = %v, want the profile's own value", got["model"])
	}
	tui, ok := got["tui"].(map[string]any)
	if !ok || tui["model_availability_nux"] != true {
		t.Errorf("tui = %v, want the profile's own table", got["tui"])
	}
	if got["sandbox_mode"] != "workspace-write" {
		t.Errorf("sandbox_mode = %v: a policy key must still arrive", got["sandbox_mode"])
	}
	for _, key := range []string{"model", "tui"} {
		if slices.Contains(result.Synced, key) {
			t.Errorf("%q belongs to the profile and must never be reported as synced", key)
		}
	}
}

// A table only the profile has may be one the user set inside the profile or one
// a newer CLI wrote. Either way the sync cannot judge it, so it stays.
func TestSeedSyncKeepsTablesOnlyTheCodexProfileHas(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "sandbox_mode = \"workspace-write\"\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"),
		"[mcp_servers.only_in_this_profile]\ncommand = \"profile-server\"\n")

	if _, err := EnsureProfileDir("codex", profile); err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	got := readProfileTOML(t, profile, "config.toml")
	servers, ok := got["mcp_servers"].(map[string]any)
	if !ok {
		t.Fatalf("mcp_servers = %v: a table only the profile had was dropped by the sync", got["mcp_servers"])
	}
	if _, ok := servers["only_in_this_profile"]; !ok {
		t.Errorf("mcp_servers = %v: the profile's own server was dropped", servers)
	}
	if got["sandbox_mode"] != "workspace-write" {
		t.Errorf("sandbox_mode = %v: a policy key must still arrive", got["sandbox_mode"])
	}
}

// Rewriting a config.toml costs the user its comments and its key order, so a
// pass that has nothing to change must not write at all — and then a profile
// that agrees with the default keeps both, forever.
func TestSeedSyncDoesNotRewriteAnAgreeingCodexProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "approval_policy = \"on-request\"\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	dest := filepath.Join(profile, "config.toml")
	// A comment, a literal string and extra spacing: the same meaning, other
	// bytes than this package would write.
	body := "# why this profile approves the way it does\napproval_policy   =   'on-request'\n"
	writeFile(t, dest, body)
	before, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if len(result.Synced) != 0 {
		t.Errorf("nothing differs, yet the pass reported %v as synced", result.Synced)
	}
	after, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}
	if !after.ModTime().Equal(before.ModTime()) {
		t.Error("config.toml was rewritten even though nothing differs")
	}
	kept, err := os.ReadFile(dest)
	if err != nil || string(kept) != body {
		t.Fatalf("config.toml = %q, %v; an untouched file keeps its comments", kept, err)
	}
}

// Same reason as the JSON side: reading the link instead of the file behind it
// would make the default look empty, and an empty default means emptying the
// profile to match.
func TestSeedSyncResolvesASymlinkedCodexDefault(t *testing.T) {
	home := isolateVendorHome(t)
	codexDir := filepath.Join(home, ".codex")
	realDefault := filepath.Join(t.TempDir(), "synced-config", "config.toml")
	writeFile(t, realDefault, "approval_policy = \"on-request\"\n")
	if err := os.MkdirAll(codexDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(realDefault, filepath.Join(codexDir, "config.toml")); err != nil {
		t.Skipf("cannot create the default-side symlink this test needs: %v", err)
	}

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"), "[tui]\nmodel_availability_nux = true\n")

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if slices.Contains(result.Failed, "config.toml") {
		t.Fatalf("a symlinked default could not be read; failed=%v", result.Failed)
	}
	got := readProfileTOML(t, profile, "config.toml")
	if got["approval_policy"] != "on-request" {
		t.Errorf("approval_policy = %v, want the value behind the symlink", got["approval_policy"])
	}
}

// A default nobody can parse must leave the profile exactly as it was, or a
// half-written save on the default side takes a working profile down with it.
func TestSeedSyncLeavesTheCodexProfileAloneWhenTheDefaultIsBroken(t *testing.T) {
	home := isolateVendorHome(t)
	// Unterminated table header: the shape a half-written save leaves behind.
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), "[mcp_servers.demo\ncommand = \"demo-server\"\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	dest := filepath.Join(profile, "config.toml")
	body := "approval_policy = \"never\"\n"
	writeFile(t, dest, body)

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if !slices.Contains(result.Failed, "config.toml") {
		t.Errorf("an unparsable default must be reported as failed; failed=%v synced=%v",
			result.Failed, result.Synced)
	}
	kept, err := os.ReadFile(dest)
	if err != nil || string(kept) != body {
		t.Fatalf("config.toml = %q, %v", kept, err)
	}
}

// many-ai-cli writes a [[hooks.Stop]] block into the *default* config.toml while
// a Codex session runs and removes it afterwards, and the command in it carries
// the Hub token. Syncing hooks as policy would take a copy of that block into
// whichever profile happened to be prepared meanwhile — and a key only the
// profile has is never removed again, so the dead session's hook would stay and
// keep firing.
func TestSeedSyncNeverCarriesTheCodexStopHookIntoAProfile(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".codex", "config.toml"),
		"approval_policy = \"on-request\"\n\n[[hooks.Stop]]\n"+
			"command = \"MANY_AI_CLI_HUB_TOKEN=synthetic-value-for-this-test many-ai-cli usage-relay --provider codex\"\n")

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"), "[tui]\nmodel_availability_nux = true\n")

	result, err := EnsureProfileDir("codex", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	// Positive control: the pass did run and did carry the policy key.
	if !slices.Contains(result.Synced, "approval_policy") {
		t.Fatalf("the sync did not run; synced=%v failed=%v", result.Synced, result.Failed)
	}
	if slices.Contains(result.Synced, "hooks") {
		t.Errorf("hooks was reported as synced; synced=%v", result.Synced)
	}
	got := readProfileTOML(t, profile, "config.toml")
	if _, ok := got["hooks"]; ok {
		t.Error("the Stop hook many-ai-cli injects into the default reached a profile")
	}
	raw, err := os.ReadFile(filepath.Join(profile, "config.toml"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "MANY_AI_CLI_HUB_TOKEN") {
		t.Errorf("the profile's config.toml holds the injected hook command:\n%s", raw)
	}
}

// The profile's copy is written back from a parsed document, so everything the
// merge did not touch has to survive that round trip: a second-level table, a
// table inside it, and an array of tables. Flattening any of them into inline
// tables would still parse, but it would no longer be the file the user wrote.
func TestSeedSyncKeepsNestedTablesAndArraysOfTables(t *testing.T) {
	home := isolateVendorHome(t)
	defaults := strings.Join([]string{
		`model = "the-default-model"`,
		``,
		`[features]`,
		`web_search = true`,
		``,
		`[[marketplaces]]`,
		`name = "first"`,
		`url = "https://example.com/first"`,
		``,
		`[[marketplaces]]`,
		`name = "second"`,
		`url = "https://example.com/second"`,
		``,
		`[mcp_servers.demo]`,
		`command = "demo-server"`,
		`args = ["--flag", "value"]`,
		``,
		`[mcp_servers.demo.env]`,
		`DEMO_MODE = "1"`,
		``,
	}, "\n")
	writeFile(t, filepath.Join(home, ".codex", "config.toml"), defaults)

	profile := filepath.Join(t.TempDir(), "codex", "work")
	writeFile(t, filepath.Join(profile, "config.toml"), "[tui]\nmodel_availability_nux = true\n")

	if _, err := EnsureProfileDir("codex", profile); err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	var want map[string]any
	if err := toml.Unmarshal([]byte(defaults), &want); err != nil {
		t.Fatalf("the test's own default is not valid TOML: %v", err)
	}
	got := readProfileTOML(t, profile, "config.toml")
	for _, key := range []string{"features", "marketplaces", "mcp_servers"} {
		if !reflect.DeepEqual(got[key], want[key]) {
			t.Errorf("%s did not survive the merge unchanged:\n got %#v\nwant %#v", key, got[key], want[key])
		}
	}
	// model is one of Codex's own keys, so an existing profile is not given the
	// default's value for it.
	if _, ok := got["model"]; ok {
		t.Errorf("model = %v: a state key must not be carried into an existing profile", got["model"])
	}
	raw, err := os.ReadFile(filepath.Join(profile, "config.toml"))
	if err != nil {
		t.Fatal(err)
	}
	for _, header := range []string{"[[marketplaces]]", "[mcp_servers.demo.env]"} {
		if !strings.Contains(string(raw), header) {
			t.Errorf("%s was flattened out of the written document:\n%s", header, raw)
		}
	}
}

// The adapter's entry list is shared by every profile of that provider. If one
// profile's settings edited it in place, the next profile in the same pass would
// inherit them — a silent cross-profile leak of exactly the setting the user
// wrote to keep two profiles apart.
func TestApplyProfileSyncOverridesDoesNotMutateTheAdapterList(t *testing.T) {
	isolateVendorHome(t)
	entries := seedEntriesFor("claude")
	before := make([]SeedEntry, len(entries))
	copy(before, entries)

	off := false
	_ = ApplyProfileSyncOverrides(entries, config.SubscriptionProfile{
		ID: "work", SettingsSync: &off, ProfileOwnedKeys: []string{"enabledPlugins"},
	})
	for i := range entries {
		if entries[i].Kind != before[i].Kind {
			t.Errorf("entry %q had its kind rewritten in place", entries[i].Dest)
		}
		if !slices.Equal(entries[i].StateKeys, before[i].StateKeys) {
			t.Errorf("entry %q had its state keys rewritten in place: %v", entries[i].Dest, entries[i].StateKeys)
		}
	}
	// A profile that configured nothing gets the list back untouched.
	plain := ApplyProfileSyncOverrides(entries, config.SubscriptionProfile{ID: "plain"})
	for i := range plain {
		if plain[i].Kind != before[i].Kind || !slices.Equal(plain[i].StateKeys, before[i].StateKeys) {
			t.Errorf("a profile with no sync settings changed entry %q", plain[i].Dest)
		}
	}
}

// When the user's default CLAUDE.md is itself a symlink (e.g. synced from a
// different real path), the profile must get a symlink to the same resolved
// target rather than a snapshot copy, so later edits to the default reach the
// profile with no re-seed.
func TestSeedLinksRuleFileWhenDefaultIsSymlink(t *testing.T) {
	home := isolateVendorHome(t)
	claudeDir := filepath.Join(home, ".claude")

	realTarget := filepath.Join(t.TempDir(), "real-claude-md", "CLAUDE.md")
	writeFile(t, realTarget, "# the one true rules file\n")

	if err := os.MkdirAll(claudeDir, 0o700); err != nil {
		t.Fatal(err)
	}
	linked := true
	if err := os.Symlink(realTarget, filepath.Join(claudeDir, "CLAUDE.md")); err != nil {
		linked = false
	}

	profile := filepath.Join(t.TempDir(), "claude", "work")
	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if !appliedContains(result, "CLAUDE.md") {
		t.Fatalf("CLAUDE.md was not seeded; applied=%v failed=%v", result.Applied, result.Failed)
	}

	dest := filepath.Join(profile, "CLAUDE.md")
	info, err := os.Lstat(dest)
	if err != nil {
		t.Fatalf("Lstat(profile CLAUDE.md): %v", err)
	}

	if !linked {
		// This environment cannot create file symlinks at all (os.Symlink itself
		// failed on the default side), so mirrorSeedFile never saw a symlink
		// source and behaved like a plain copy. Confirm that instead.
		if info.Mode()&os.ModeSymlink != 0 {
			t.Fatal("profile CLAUDE.md is a symlink even though this environment could not create one")
		}
		body, err := os.ReadFile(dest)
		if err != nil || string(body) != "# the one true rules file\n" {
			t.Fatalf("CLAUDE.md copy = %q, %v", body, err)
		}
		return
	}

	if info.Mode()&os.ModeSymlink == 0 {
		t.Fatal("profile CLAUDE.md must be a symlink when the default is a symlink")
	}
	resolved, err := filepath.EvalSymlinks(dest)
	if err != nil {
		t.Fatalf("EvalSymlinks(profile CLAUDE.md): %v", err)
	}
	wantTarget, err := filepath.EvalSymlinks(realTarget)
	if err != nil {
		t.Fatalf("EvalSymlinks(realTarget): %v", err)
	}
	if resolved != wantTarget {
		t.Errorf("profile CLAUDE.md resolves to %q, want %q", resolved, wantTarget)
	}
	body, err := os.ReadFile(dest)
	if err != nil || string(body) != "# the one true rules file\n" {
		t.Fatalf("reading through the profile symlink = %q, %v", body, err)
	}
	if appliedContains(result, "CLAUDE.md") && slices.Contains(result.Degraded, "CLAUDE.md") {
		t.Error("a successful symlink must not also be reported as degraded")
	}
}

// A plain, non-symlinked default CLAUDE.md must still be copied, unchanged
// from the behavior before SeedMirrorFile existed.
func TestSeedCopiesRuleFileWhenDefaultIsRegular(t *testing.T) {
	home := isolateVendorHome(t)
	writeFile(t, filepath.Join(home, ".claude", "CLAUDE.md"), "# plain rules\n")

	profile := filepath.Join(t.TempDir(), "claude", "work")
	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if !appliedContains(result, "CLAUDE.md") {
		t.Fatalf("CLAUDE.md was not seeded; applied=%v failed=%v", result.Applied, result.Failed)
	}
	if slices.Contains(result.Degraded, "CLAUDE.md") {
		t.Error("copying a regular file must never be reported as degraded")
	}
	dest := filepath.Join(profile, "CLAUDE.md")
	info, err := os.Lstat(dest)
	if err != nil {
		t.Fatalf("Lstat: %v", err)
	}
	if info.Mode()&os.ModeSymlink != 0 {
		t.Fatal("profile CLAUDE.md must be a plain file when the default is a plain file")
	}
	body, err := os.ReadFile(dest)
	if err != nil || string(body) != "# plain rules\n" {
		t.Fatalf("CLAUDE.md = %q, %v", body, err)
	}
}

// When creating the symlink fails (Windows without Developer Mode, or any
// other environment-specific restriction), mirrorSeedFile must fall back to a
// plain copy and report the entry as degraded rather than failing outright.
func TestSeedFallsBackToCopyWhenSymlinkFails(t *testing.T) {
	home := isolateVendorHome(t)
	claudeDir := filepath.Join(home, ".claude")

	realTarget := filepath.Join(t.TempDir(), "real-claude-md", "CLAUDE.md")
	writeFile(t, realTarget, "# the one true rules file\n")
	if err := os.MkdirAll(claudeDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(realTarget, filepath.Join(claudeDir, "CLAUDE.md")); err != nil {
		t.Skipf("cannot create the default-side symlink this test needs: %v", err)
	}

	original := symlinkFile
	symlinkFile = func(string, string) error { return errors.New("simulated: no privilege to create a file symlink") }
	t.Cleanup(func() { symlinkFile = original })

	profile := filepath.Join(t.TempDir(), "claude", "work")
	result, err := EnsureProfileDir("claude", profile)
	if err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	if !appliedContains(result, "CLAUDE.md") {
		t.Fatalf("CLAUDE.md was not seeded even via the fallback; applied=%v failed=%v", result.Applied, result.Failed)
	}
	if !slices.Contains(result.Degraded, "CLAUDE.md") {
		t.Errorf("a failed symlink attempt must be reported as degraded; degraded=%v", result.Degraded)
	}

	dest := filepath.Join(profile, "CLAUDE.md")
	info, err := os.Lstat(dest)
	if err != nil {
		t.Fatalf("Lstat: %v", err)
	}
	if info.Mode()&os.ModeSymlink != 0 {
		t.Fatal("profile CLAUDE.md must be a regular file after the symlink fallback, not a symlink")
	}
	body, err := os.ReadFile(dest)
	if err != nil || string(body) != "# the one true rules file\n" {
		t.Fatalf("CLAUDE.md copy after fallback = %q, %v", body, err)
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

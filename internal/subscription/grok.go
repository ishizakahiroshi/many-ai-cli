package subscription

import (
	"context"
	"path/filepath"
	"strings"
)

// GrokHomeEnv is the environment variable Grok Build CLI reads to select its
// home directory (default ~/.grok). Verified on grok 1.0.4 (2026-08-17):
// `grok du` reports "$GROK_HOME" when it is set, and with it pointed at an empty
// directory `grok models` answers "You are not authenticated." while the default
// home stays signed in. The credential itself is $GROK_HOME/auth.json, so moving
// the home moves the login.
const GrokHomeEnv = "GROK_HOME"

func init() { Register(grokAdapter{}) }

type grokAdapter struct{}

func (grokAdapter) Provider() string { return "grok" }

func (grokAdapter) EnvVar() string { return GrokHomeEnv }

func (grokAdapter) LaunchEnv(profileDir string) []string {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	return []string{GrokHomeEnv + "=" + profileDir}
}

func (grokAdapter) LoginArgs() []string { return []string{"login"} }

func (a grokAdapter) Status(ctx context.Context, profileDir string) (Status, error) {
	// `grok models` is the lightest command that reports authentication state.
	// There is no dedicated status subcommand, and its exit code is 0 either way,
	// so the answer has to come from the text.
	out, _, err := runVendorCLI(ctx, "grok", []string{"models"}, a.LaunchEnv(profileDir))
	if err != nil {
		return Status{}, err
	}
	return parseGrokModelsStatus(out), nil
}

// grokLoginMethods is a whitelist of sign-in sources worth surfacing. Matching
// against a fixed list (rather than extracting whatever follows "logged in with")
// keeps anything account-shaped out of the Status, even if the wording changes.
var grokLoginMethods = []string{"grok.com", "x.ai"}

func parseGrokModelsStatus(out string) Status {
	lower := strings.ToLower(out)
	if strings.Contains(lower, "not authenticated") || strings.Contains(lower, "not logged in") {
		return Status{LoggedIn: false}
	}
	if !strings.Contains(lower, "logged in") && !strings.Contains(lower, "authenticated") {
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	for _, method := range grokLoginMethods {
		if strings.Contains(lower, method) {
			status.Method = method
			break
		}
	}
	return status
}

// SeedEntries lists what a Grok profile inherits from $GROK_HOME.
//
// trusted_folders.toml is included for the same reason as Codex's trust list:
// a profile that forgets it re-asks for folder trust the user already granted,
// and answering the same prompt repeatedly is how people learn to approve
// without reading. auth.json stays out — it is the credential.
func (grokAdapter) SeedEntries() []SeedEntry {
	dir := vendorDefaultDir(GrokHomeEnv, ".grok")
	if dir == "" {
		return nil
	}
	return []SeedEntry{
		{Source: filepath.Join(dir, "AGENTS.md"), Dest: "AGENTS.md",
			Kind: SeedMirrorFile, Label: "共通ルール（AGENTS.md）"},
		{Source: filepath.Join(dir, "config.toml"), Dest: "config.toml",
			Kind: SeedCopyFile, Label: "設定（config.toml・既定モデルを含む）"},
		{Source: filepath.Join(dir, "trusted_folders.toml"), Dest: "trusted_folders.toml",
			Kind: SeedCopyFile, Label: "信頼済みフォルダ（trusted_folders.toml）"},
		{Source: filepath.Join(dir, "skills"), Dest: "skills",
			Kind: SeedLinkDir, Label: "スキル（skills/）"},
	}
}

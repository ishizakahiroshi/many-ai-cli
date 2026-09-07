package subscription

import (
	"context"
	"path/filepath"
	"strings"
)

// CodexHomeEnv is the environment variable Codex CLI reads to select its home
// directory. Verified on codex-cli 0.147.0 (2026-08-17): pointing it at an empty
// directory makes `codex login status` print "Not logged in" and exit 1 while the
// default home stays signed in, so two directories hold two independent logins.
//
// many-ai-cli never reads or writes $CODEX_HOME/auth.json. Choosing the official
// directory switch over rewriting the auth file is deliberate: the file's shape
// (auth_mode / OPENAI_API_KEY / tokens / last_refresh) is an internal detail that
// changes with CLI releases, and touching it would make many-ai-cli a token store.
const CodexHomeEnv = "CODEX_HOME"

func init() { Register(codexAdapter{}) }

type codexAdapter struct{}

func (codexAdapter) Provider() string { return "codex" }

func (codexAdapter) EnvVar() string { return CodexHomeEnv }

func (codexAdapter) LaunchEnv(profileDir string) []string {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	return []string{CodexHomeEnv + "=" + profileDir}
}

func (codexAdapter) LoginArgs() []string { return []string{"login"} }

func (a codexAdapter) Status(ctx context.Context, profileDir string) (Status, error) {
	out, code, err := runVendorCLI(ctx, "codex", []string{"login", "status"}, a.LaunchEnv(profileDir))
	if err != nil {
		return Status{}, err
	}
	return parseCodexLoginStatus(out, code), nil
}

// parseCodexLoginStatus reads only the shape of the answer, never the account
// line itself: `codex login status` prints the signed-in account, so the raw
// output must not travel any further than this function.
//
// An API-key login is reported as signed in with method "api-key" rather than
// hidden. Subscription profiles target the ChatGPT sign-in, and silently showing
// an API-key profile as if it were a subscription would hide metered billing.
func parseCodexLoginStatus(out string, exitCode int) Status {
	lower := strings.ToLower(out)
	if strings.Contains(lower, "not logged in") {
		return Status{LoggedIn: false}
	}
	if exitCode != 0 && !strings.Contains(lower, "logged in") {
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	switch {
	case strings.Contains(lower, "chatgpt"):
		status.Method = "chatgpt"
	case strings.Contains(lower, "api key"), strings.Contains(lower, "api-key"):
		status.Method = "api-key"
	}
	return status
}

// SeedEntries lists what a Codex profile inherits from $CODEX_HOME.
//
// config.toml is carried whole rather than key by key. It holds approval_policy,
// sandbox_mode and the per-project trust_level list, and a profile without them
// re-asks for every trust decision the user already made. Two things about that
// are worth knowing rather than discovering:
//
//   - The file can hold an [mcp_servers.*.env] block, so a key written there is
//     copied too. It lands in the profile directory, which already holds
//     auth.json and is already owner-only, so nothing becomes readable to anyone
//     who could not already read it.
//   - Absolute paths inside it are copied verbatim. This user's file sets
//     CODEX_HOME for an MCP server, which then points at the default home until
//     Codex rewrites that block.
//
// auth.json is never seeded: it is the credential, and separating it is the
// entire reason profiles exist.
//
// skills/ is left alone on purpose. Codex ships a bundled skills/.system
// directory, and linking the whole directory would hide it. The user-level
// shelf reaches Codex through ~/.agents/skills, which CODEX_HOME does not move.
func (codexAdapter) SeedEntries() []SeedEntry {
	dir := vendorDefaultDir(CodexHomeEnv, ".codex")
	if dir == "" {
		return nil
	}
	return []SeedEntry{
		{Source: filepath.Join(dir, "AGENTS.md"), Dest: "AGENTS.md",
			Kind: SeedMirrorFile, Label: "共通ルール（AGENTS.md）"},
		{Source: filepath.Join(dir, "config.toml"), Dest: "config.toml",
			Kind: SeedCopyFile, Label: "設定（config.toml・承認ポリシー / 信頼済みフォルダを含む）"},
		{Source: filepath.Join(dir, "prompts"), Dest: "prompts",
			Kind: SeedLinkDir, Label: "カスタム スラッシュコマンド（prompts/）"},
	}
}

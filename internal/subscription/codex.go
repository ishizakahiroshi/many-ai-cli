package subscription

import (
	"context"
	"os"
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

// CodexConfigFileFromEnv reports which config.toml a Codex process started with
// env ("KEY=VALUE" entries, normally a child's spawn env) will read. Like
// ClaudeStateFileFromEnv, and unlike vendorDefaultDir, it keeps a CODEX_HOME
// that points inside the profiles tree: for a profile child that is the file
// that matters.
func CodexConfigFileFromEnv(env []string) string {
	value, _ := envSliceValue(env, CodexHomeEnv)
	if dir := strings.TrimSpace(value); dir != "" {
		return filepath.Join(dir, "config.toml")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".codex", "config.toml")
}

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

// codexProfileStateKeys are the config.toml tables a profile owns, i.e. the ones
// Codex writes for itself and the sync must not take back from the user's
// default configuration. Everything else in the file — approval_policy,
// sandbox_mode, mcp_servers, features, the notify and shell policies — is policy
// the user keeps in one place, and the default is its source of truth.
//
// 2026-09-21 に既定 ~/.codex/config.toml と profile 2 本の top-level 鍵を比較して
// 確定（値は見ていない）。profile 側で違っていたのは projects / tui / notice /
// windows と model_reasoning_effort で、2 段目はいずれも CLI が書く確認済みフラグか
// 信頼したフォルダだった。model は /model が model_reasoning_effort と対で書くため
// 同じ扱いにしている。
//
// hooks is on this list for a different reason, and it is the one entry here
// that is not about the CLI's own bookkeeping: many-ai-cli itself writes a
// [[hooks.Stop]] block into the *default* config.toml while any Codex session is
// running (wrapper.InjectCodexStopHook) and removes it when the last one exits.
// The command in that block carries the Hub token as an environment prefix. If
// hooks were policy, a profile prepared while the block is in place would take a
// copy of it — and a key only the profile has is never removed again, so the
// stale hook would stay there and fire with a dead session's token from then on.
// A user who does want their own hooks in every profile can say so per profile
// with default_wins_keys: [hooks] in config.yaml.
var codexProfileStateKeys = []string{
	"projects",
	"tui",
	"notice",
	"windows",
	"model",
	"model_reasoning_effort",
	"hooks",
}

// SeedEntries lists what a Codex profile inherits from $CODEX_HOME.
//
// config.toml is kept in step with the default on every pass rather than copied
// once. It holds approval_policy, sandbox_mode, the MCP servers and the feature
// flags, and a profile that took its copy months ago stops at whatever the file
// said then with nothing on screen to say so. The tables Codex writes for itself
// stay the profile's (codexProfileStateKeys), including the per-project trust
// levels, so no trust decision the user already made is re-asked. Three things
// about that are worth knowing rather than discovering:
//
//   - The file can hold an [mcp_servers.*.env] block, so a key written there is
//     carried too. It lands in the profile directory, which already holds
//     auth.json and is already owner-only, so nothing becomes readable to anyone
//     who could not already read it.
//   - Absolute paths inside it are carried verbatim. A default that points an
//     MCP server at CODEX_HOME keeps pointing at the default home until Codex
//     rewrites that block.
//   - The profile's copy is written back from a parsed document, so a pass that
//     changes anything drops that copy's comments and sorts its keys. The
//     default side is only ever read and keeps both.
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
			Kind:      SeedSyncFile,
			StateKeys: codexProfileStateKeys,
			Label:     "設定（config.toml・起動時に既定と同期。信頼済みフォルダ等は profile が持つ）"},
		{Source: filepath.Join(dir, "prompts"), Dest: "prompts",
			Kind: SeedLinkDir, Label: "カスタム スラッシュコマンド（prompts/）"},
	}
}

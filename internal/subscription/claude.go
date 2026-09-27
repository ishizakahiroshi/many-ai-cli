package subscription

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ClaudeConfigDirEnv is the environment variable Claude Code reads to select its
// configuration directory. Verified on Claude Code 2.1.233 (2026-08-17):
// pointing it at an empty directory makes `claude auth status` report
// loggedIn:false while the default profile stays signed in, so two sessions with
// two directories hold two independent logins.
const ClaudeConfigDirEnv = "CLAUDE_CONFIG_DIR"

func init() { Register(claudeAdapter{}) }

type claudeAdapter struct{}

func (claudeAdapter) Provider() string { return "claude" }

func (claudeAdapter) EnvVar() string { return ClaudeConfigDirEnv }

func (claudeAdapter) LaunchEnv(profileDir string) []string {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	return []string{ClaudeConfigDirEnv + "=" + profileDir}
}

func (claudeAdapter) LoginArgs() []string { return []string{"auth", "login"} }

// claudeAuthStatus mirrors the JSON printed by `claude auth status`.
//
// The command also prints email / orgId / orgName. Those fields are
// deliberately absent here: the struct is the boundary that keeps account
// identity out of the Hub API and the browser. Profiles are identified by the
// name the user gave them.
type claudeAuthStatus struct {
	LoggedIn         bool   `json:"loggedIn"`
	AuthMethod       string `json:"authMethod"`
	SubscriptionType string `json:"subscriptionType"`
}

func (a claudeAdapter) Status(ctx context.Context, profileDir string) (Status, error) {
	out, _, err := runVendorCLI(ctx, "claude", []string{"auth", "status"}, a.LaunchEnv(profileDir))
	if err != nil {
		return Status{}, err
	}
	// 未ログイン時は exit code 1 だが stdout には JSON が出るので、終了コードでは
	// なく本文で判定する。
	var parsed claudeAuthStatus
	if jsonErr := json.Unmarshal([]byte(strings.TrimSpace(out)), &parsed); jsonErr != nil {
		// 出力そのものは載せない（アカウント情報を含みうる）。
		return Status{}, fmt.Errorf("could not read `claude auth status` output")
	}
	if !parsed.LoggedIn {
		return Status{LoggedIn: false}, nil
	}
	return Status{
		LoggedIn: true,
		Plan:     parsed.SubscriptionType,
		Method:   parsed.AuthMethod,
	}, nil
}

// claudeDefaultConfigDir is where Claude Code keeps its configuration when no
// profile is selected.
func claudeDefaultConfigDir() string {
	return vendorDefaultDir(ClaudeConfigDirEnv, ".claude")
}

// claudeDefaultStateFile locates the default .claude.json — the one Claude
// Code uses when no profile is selected. defaultHomeFromEnv drops a
// CLAUDE_CONFIG_DIR that points inside many-ai-cli's own profiles tree (a Hub
// started from a profile session inherits one), because that is a profile, not
// the default this file seeds profiles from.
func claudeDefaultStateFile() string {
	return claudeStateFileIn(defaultHomeFromEnv(ClaudeConfigDirEnv))
}

// ClaudeStateFileFromEnv answers a different question from
// claudeDefaultStateFile: which .claude.json will a Claude Code process started
// with env actually read? env is a "KEY=VALUE" slice (the shape of
// os.Environ() and exec.Cmd.Env), normally the one a child is about to be
// spawned with.
//
// internal/clitrust uses it to record folder trust for a child before the
// child exists. A profile child's CLAUDE_CONFIG_DIR points inside the profiles
// tree, and that is exactly the file the child will read — so unlike
// claudeDefaultStateFile, this function must not drop such a value. Dropping
// it would write the trust into ~/.claude.json while the child reads its
// profile's file, and the trust prompt would appear anyway.
func ClaudeStateFileFromEnv(env []string) string {
	value, _ := envSliceValue(env, ClaudeConfigDirEnv)
	return claudeStateFileIn(strings.TrimSpace(value))
}

// claudeStateFileIn holds the placement rule, which moves: with no
// CLAUDE_CONFIG_DIR the file sits *beside* ~/.claude as ~/.claude.json, and
// with one it sits *inside* that directory. Getting this wrong reads nothing
// and seeds nothing, silently, which is the failure mode this whole file
// exists to remove.
func claudeStateFileIn(configDir string) string {
	if configDir != "" {
		return filepath.Join(configDir, ".claude.json")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".claude.json")
}

// envSliceValue looks up key in an environment slice shaped like os.Environ()
// or exec.Cmd.Env ("KEY=VALUE" entries). The last matching entry wins, the same
// as when the OS or os/exec applies a slice with a repeated key.
func envSliceValue(env []string, key string) (string, bool) {
	prefix := key + "="
	value, found := "", false
	for _, entry := range env {
		if strings.HasPrefix(entry, prefix) {
			value = entry[len(prefix):]
			found = true
		}
	}
	return value, found
}

// claudeCarriedStateKeys are the only .claude.json keys a new profile inherits.
//
// The file mixes account identity (oauthAccount, userID, machineID) with
// per-machine caches and with a handful of real preferences. Copying it whole
// would hand the default account's identity to a profile that is supposed to
// hold a different account, so the entries are named one at a time and their
// values are copied as opaque JSON.
//
// Both keys here are about whether Claude Code may drive the user's Chrome.
// That is a preference about a feature, unrelated to which account is signed
// in, and its absence is what started this work: browser tools were off in
// every subscription session with nothing on screen to explain why.
var claudeCarriedStateKeys = []string{
	"claudeInChromeDefaultEnabled",
	"hasCompletedClaudeInChromeOnboarding",
}

// claudeProfileStateKeys are the settings.json keys a profile owns, i.e. the
// ones Claude Code writes for itself and the sync must not take back from the
// user's default configuration. Everything else in the file — hooks,
// permissions, env, the feature switches — is policy the user keeps in one
// place, and the default is its source of truth.
//
// 初期値。modelSettings / tui / autoMode は CLI が書くことを観測済み、残りは
// /config の項目名からの推定。
var claudeProfileStateKeys = []string{
	"autoMode",
	"modelSettings",
	"tui",
	"theme",
	"effortLevel",
	"skipDangerousModePermissionPrompt",
	"skipAutoPermissionPrompt",
	"skipWorkflowUsageWarning",
	"agentPushNotifEnabled",
	"voice",
	"voiceEnabled",
	"autoUpdatesChannel",
	"switchModelsOnFlag",
}

// SeedEntries lists what a Claude Code profile inherits from the user's own
// configuration. Everything not named here — credentials, session history,
// per-project state, caches — stays separate, which is the point of profiles.
func (claudeAdapter) SeedEntries() []SeedEntry {
	dir := claudeDefaultConfigDir()
	if dir == "" {
		return nil
	}
	entries := []SeedEntry{
		{Source: filepath.Join(dir, "CLAUDE.md"), Dest: "CLAUDE.md",
			Kind: SeedMirrorFile, Label: "共通ルール（CLAUDE.md）"},
		{Source: filepath.Join(dir, "settings.json"), Dest: "settings.json",
			Kind:      SeedSyncFile,
			StateKeys: claudeProfileStateKeys,
			Label:     "ユーザー設定（settings.json・起動時に既定と同期）"},
		{Source: filepath.Join(dir, "commands"), Dest: "commands",
			Kind: SeedLinkDir, Label: "スラッシュコマンド（commands/）"},
		{Source: filepath.Join(dir, "skills"), Dest: "skills",
			Kind: SeedLinkDir, Label: "スキル（skills/）"},
		{Source: filepath.Join(dir, "agents"), Dest: "agents",
			Kind: SeedLinkDir, Label: "サブエージェント（agents/）"},
	}
	if state := claudeDefaultStateFile(); state != "" {
		entries = append(entries, SeedEntry{
			Source: state, Dest: ".claude.json", Kind: SeedJSONKeys,
			Keys:  claudeCarriedStateKeys,
			Label: "ブラウザ操作の既定（.claude.json の名指しキーのみ）",
		})
	}
	return entries
}

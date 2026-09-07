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

// claudeDefaultStateFile locates the default .claude.json.
//
// It moves: with CLAUDE_CONFIG_DIR unset the file sits *beside* ~/.claude as
// ~/.claude.json, and with it set the file sits *inside* the directory. Getting
// this wrong reads nothing and seeds nothing, silently, which is the failure
// mode this whole file exists to remove.
func claudeDefaultStateFile() string {
	if dir := defaultHomeFromEnv(ClaudeConfigDirEnv); dir != "" {
		return filepath.Join(dir, ".claude.json")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".claude.json")
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
			Kind: SeedCopyFile, Label: "ユーザー設定（settings.json・承認設定 / hooks を含む）"},
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

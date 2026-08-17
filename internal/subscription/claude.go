package subscription

import (
	"context"
	"encoding/json"
	"fmt"
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

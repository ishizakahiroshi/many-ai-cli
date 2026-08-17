package subscription

import (
	"context"
	"regexp"
	"strconv"
	"strings"
)

// OpenCodeDataHomeEnv is the variable OpenCode uses to locate its data
// directory. Verified on the installed OpenCode (2026-08-17): with
// XDG_DATA_HOME pointed at an empty directory, `opencode providers list`
// reports "0 credentials" at <dir>/opencode/auth.json while the default
// location keeps its credentials.
//
// Caveat, deliberately not hidden: XDG_DATA_HOME is a *generic* variable, not an
// OpenCode-specific one. Every XDG-aware tool the agent runs inside that session
// inherits it, so its data lands under the profile directory for the life of the
// session. OpenCode has no dedicated variable today; if it grows one, switch to
// that and drop this note. The user's shell is never touched — only the session's
// own child processes.
//
// OpenCode reads its *config* from XDG_CONFIG_HOME, which is left alone. Unlike
// Claude and Codex, an OpenCode profile therefore keeps the user's global config
// and skills and only splits credentials.
const OpenCodeDataHomeEnv = "XDG_DATA_HOME"

func init() { Register(openCodeAdapter{}) }

type openCodeAdapter struct{}

func (openCodeAdapter) Provider() string { return "opencode" }

func (openCodeAdapter) EnvVar() string { return OpenCodeDataHomeEnv }

func (openCodeAdapter) LaunchEnv(profileDir string) []string {
	if strings.TrimSpace(profileDir) == "" {
		return nil
	}
	// OpenCode appends its own "opencode" directory under this path, so the
	// profile directory is handed over as-is.
	return []string{OpenCodeDataHomeEnv + "=" + profileDir}
}

func (openCodeAdapter) LoginArgs() []string { return []string{"providers", "login"} }

func (a openCodeAdapter) Status(ctx context.Context, profileDir string) (Status, error) {
	out, _, err := runVendorCLI(ctx, "opencode", []string{"providers", "list"}, a.LaunchEnv(profileDir))
	if err != nil {
		return Status{}, err
	}
	return parseOpenCodeProvidersList(out), nil
}

var openCodeCredentialCountRe = regexp.MustCompile(`(\d+)\s+credentials?`)

// parseOpenCodeProvidersList reads only the credential count and whether the
// OpenCode Go entry is present. The listing shows provider names and auth kinds
// but no secret values; nothing beyond these two facts leaves this function.
func parseOpenCodeProvidersList(out string) Status {
	lower := strings.ToLower(out)
	count := -1
	if m := openCodeCredentialCountRe.FindStringSubmatch(lower); len(m) == 2 {
		if n, err := strconv.Atoi(m[1]); err == nil {
			count = n
		}
	}
	if count == 0 {
		return Status{LoggedIn: false}
	}
	if count < 0 && !strings.Contains(lower, "credential") {
		// 想定外の出力形。ログイン済みと決めつけない。
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	// 今回の対象は Go 月額契約に紐づく credential。他 provider の API キーだけが
	// 入っている profile を「サブスクリプション」と見せないよう区別する。
	if strings.Contains(lower, "opencode go") {
		status.Plan = "go"
	}
	return status
}

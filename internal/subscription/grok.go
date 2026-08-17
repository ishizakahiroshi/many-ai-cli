package subscription

import (
	"context"
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

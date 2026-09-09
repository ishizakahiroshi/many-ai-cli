// Package subscription manages multiple vendor-CLI logins ("subscription
// profiles") for a single provider.
//
// The whole design rests on one fact confirmed on real CLIs
// (docs/local/plan_multi-subscription-pool_c1_survey.md): every supported
// vendor CLI selects its configuration directory from an environment variable,
// and pointing two sessions at two directories keeps their logins independent.
// many-ai-cli therefore only ever creates a directory and sets one environment
// variable. It never reads, writes, parses, or stores the credential itself.
//
// The rules below used to live in CLAUDE.md; they were moved here on
// 2026-08-19 because anyone who breaks one of them is, by definition, editing
// this package. Each is pinned by a named test where that is possible.
//
//   - Do not add code that reads, writes, or parses an auth file. Sign-in state
//     comes only from the vendor CLI's own status subcommand; the file formats
//     change with CLI releases.
//   - Do not keep a token, API key, or PAT in config.yaml or any store of our
//     own. Allowing that turns many-ai-cli into a token vault, which is exactly
//     why GitHub Copilot CLI and Cursor Agent are recorded as unsupported.
//   - A spawn with no profile must build a byte-for-byte identical environment
//     (TestSubscriptionLaunchWithoutProfileLeavesEnvUnchanged,
//     TestFakeProviderWithoutProfileKeepsInheritedEnv).
//   - A spawn naming a missing or disabled profile must fail, never fall back
//     to another account or to the default login.
//   - Never swap the credentials of a live session
//     (TestLiveSessionAuthIsNeverSwapped scans the source for assignments).
//   - Remaining quota cannot be *queried* from the vendor CLIs (measured
//     2026-08-17), so there is no ReadUsage on Adapter. That is separate from
//     the three actual data paths: Claude pushes rate_limits from statusLine,
//     Codex leaves rate_limits in rollout JSONL, and Grok leaves billing data
//     in unified.jsonl. The Hub reads the latter two as local files; providers
//     without a path stay out of the usage panel rather than gaining an
//     "Unknown" column. Reading the first sentence as "no usage data exists
//     anywhere" leads to documenting a shipped feature as missing.
package subscription

import (
	"context"
	"sort"
	"sync"
)

// SessionEnvVar carries the selected profile id from the Hub to the wrapper.
//
// The wrapper reads it back and reports it during register, so the session card
// shows the profile that actually launched rather than the one the Hub meant to
// launch. It holds an opaque id, never a credential.
const SessionEnvVar = "MANY_AI_CLI_SUBSCRIPTION_ID"

// Status is the secret-free answer to "is this profile signed in?".
//
// It deliberately carries no account identity (no email, no organisation id).
// The profile's user-supplied Name is what identifies an account in the UI.
type Status struct {
	LoggedIn bool   `json:"logged_in"`
	Plan     string `json:"plan,omitempty"`
	Method   string `json:"method,omitempty"`
}

// Adapter isolates the provider-specific parts of profile handling.
//
// The interface is deliberately small. Usage/quota reading is absent because
// neither Claude Code nor Codex CLI exposes remaining quota through their CLIs
// (verified 2026-08-17); adding a method nobody can implement would only make
// every adapter carry a stub.
type Adapter interface {
	// Provider returns the many-ai-cli provider id ("claude", "codex", ...).
	Provider() string
	// EnvVar is the environment variable the vendor CLI reads to pick its
	// configuration directory. Shown in the UI so the mechanism is not magic.
	EnvVar() string
	// LaunchEnv returns the KEY=VALUE entries to overlay on a child process env
	// so the vendor CLI uses profileDir. An empty profileDir yields nil, which
	// keeps the child's environment byte-for-byte identical to before.
	LaunchEnv(profileDir string) []string
	// LoginArgs returns the vendor CLI arguments that start its own login flow.
	// many-ai-cli never implements an OAuth flow of its own.
	LoginArgs() []string
	// Status asks the vendor CLI whether profileDir holds a signed-in account.
	// Implementations must not return credential material in Status or in err.
	Status(ctx context.Context, profileDir string) (Status, error)
}

var (
	registryMu sync.RWMutex
	registry   = map[string]Adapter{}
)

// Register adds an adapter. Adapters register from init() in their own file so
// that dropping a provider file removes it from the registry with no other edit.
func Register(a Adapter) {
	if a == nil {
		return
	}
	registryMu.Lock()
	defer registryMu.Unlock()
	registry[a.Provider()] = a
}

// AdapterFor returns the adapter for provider, if one is registered.
// A missing adapter is a normal state, not an error: the Hub must start and
// spawn as usual when no provider supports profiles.
func AdapterFor(provider string) (Adapter, bool) {
	registryMu.RLock()
	defer registryMu.RUnlock()
	a, ok := registry[provider]
	return a, ok
}

// SupportedProviders returns the registered provider ids in a stable order.
func SupportedProviders() []string {
	registryMu.RLock()
	out := make([]string, 0, len(registry))
	for provider := range registry {
		out = append(out, provider)
	}
	registryMu.RUnlock()
	sort.Strings(out)
	return out
}

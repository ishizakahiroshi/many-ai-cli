package config

import (
	"fmt"
	"strings"

	"gopkg.in/yaml.v3"
)

// MaxCustomProviderIDLen bounds custom_providers[].id. Reuses the same bound
// as MaxSubscriptionIDLen: a short identifier typed by hand, not a path.
const MaxCustomProviderIDLen = 64

// BuiltinProviderIDs lists the provider identifiers many-ai-cli ships built
// in (CLAUDE.md 用語・名称 table, plus "command-code"). Keep in sync with the
// provider whitelist in internal/hub/spawn_handler.go. This is also
// doctor.providers()'s LookPath+--version probe list, which is why "shell"
// is deliberately not in it (see IsReservedProviderID) — "shell" has no real
// binary of its own to probe; resolveDefaultShell() resolves it dynamically
// to pwsh/bash/cmd at spawn time, so listing it here would make doctor
// wrongly report "shell not found in PATH".
var BuiltinProviderIDs = []string{
	"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code",
}

// IsBuiltinProviderID reports whether id (compared case-insensitively) names
// a built-in provider.
func IsBuiltinProviderID(id string) bool {
	id = NormalizeSubscriptionID(id)
	for _, b := range BuiltinProviderIDs {
		if id == b {
			return true
		}
	}
	return false
}

// reservedShellProviderID is off-limits as a custom_providers id even though
// it is not in BuiltinProviderIDs. Once spawn validation accepts dynamic
// custom ids (plan_custom-provider-spawn-execution.md), an "id: shell" entry
// would pass ValidateCustomProviderID yet get swallowed by the wrapper's
// "shell" dispatch branch — silently ignoring Command and opening an
// interactive shell instead. See IsReservedProviderID.
const reservedShellProviderID = "shell"

// IsReservedProviderID reports whether id (compared case-insensitively) may
// not be used as a custom_providers id: every BuiltinProviderIDs entry, plus
// "shell". This is a superset of IsBuiltinProviderID — it is the set
// ValidateCustomProviderID rejects, not the set doctor.providers() probes.
func IsReservedProviderID(id string) bool {
	id = NormalizeSubscriptionID(id)
	return IsBuiltinProviderID(id) || id == reservedShellProviderID
}

// CustomProvider is one user-defined AI CLI entry under config.yaml's
// `custom_providers:`. This is the only place it can be created — the Hub UI
// only displays it and lets a session spawn with it, never adds or edits one
// (docs/local/plan_provider-user-config.md decisions). Whatever Command
// launches is the user's own choice: many-ai-cli's built-in-provider
// decisions (D-01 / D-09 in reference_declined-directions.md) do not apply to
// it, and neither does the built-in approval-hook wiring.
type CustomProvider struct {
	// ID is the value a session spawns with (e.g. "my-cli"). Must not
	// collide with a BuiltinProviderIDs entry or with another
	// custom_providers id — see ValidateCustomProviderID.
	ID string `yaml:"id" json:"id"`
	// Label is the spawn-dropdown display text. Falls back to ID when empty
	// — see EffectiveLabel.
	Label string `yaml:"label,omitempty" json:"label,omitempty"`
	// Command is the command line many-ai-cli runs for this provider.
	Command string `yaml:"command" json:"command"`
	// ApprovalPatternSource optionally points at a local file or URL holding
	// this CLI's approval-prompt patterns, mirroring ApprovalPatternSources
	// for built-in providers. Empty means no approval detection.
	ApprovalPatternSource string `yaml:"approval_pattern_source,omitempty" json:"approval_pattern_source,omitempty"`
}

// EffectiveLabel returns Label, falling back to ID when Label is empty.
func (p CustomProvider) EffectiveLabel() string {
	if strings.TrimSpace(p.Label) != "" {
		return p.Label
	}
	return p.ID
}

// CustomProviders is the `custom_providers:` list.
type CustomProviders []CustomProvider

// UnmarshalYAML decodes tolerantly, for the same reason as
// SubscriptionProfiles.UnmarshalYAML: config.yaml is hand-written for this
// section, so one malformed entry must not fail the whole file (LoadOrCreate
// would back it up and regenerate a new token, changing the Hub URL). An
// entry missing id or command is dropped here.
//
// The built-in-collision and duplicate-id checks happen later, in
// EffectiveCustomProviders / Warnings(), not here — so cfg.CustomProviders
// still reflects exactly what the user wrote (Save() must round-trip it
// rather than silently deleting an entry the user typed wrong).
func (c *CustomProviders) UnmarshalYAML(value *yaml.Node) error {
	var raw []yaml.Node
	if err := value.Decode(&raw); err != nil {
		*c = nil
		return nil
	}
	out := make(CustomProviders, 0, len(raw))
	for _, node := range raw {
		var p CustomProvider
		if err := node.Decode(&p); err != nil {
			continue
		}
		if strings.TrimSpace(p.ID) == "" || strings.TrimSpace(p.Command) == "" {
			continue
		}
		out = append(out, p)
	}
	if len(out) == 0 {
		*c = nil
		return nil
	}
	*c = out
	return nil
}

// ValidateCustomProviderID reports whether id is safe to use as a spawn
// provider value and does not collide with a built-in provider. id should
// already be normalized (NormalizeSubscriptionID).
func ValidateCustomProviderID(id string) error {
	if id == "" {
		return fmt.Errorf("custom provider id is required")
	}
	if len(id) > MaxCustomProviderIDLen {
		return fmt.Errorf("custom provider id is longer than %d characters", MaxCustomProviderIDLen)
	}
	if !subscriptionIDRe.MatchString(id) {
		return fmt.Errorf("custom provider id %q may only contain lowercase letters, digits, dot, underscore and hyphen, and must start with a letter or digit", id)
	}
	if IsReservedProviderID(id) {
		return fmt.Errorf("custom provider id %q collides with a built-in provider or a reserved id", id)
	}
	return nil
}

// IsCustomProviderID reports whether id names an entry in
// EffectiveCustomProviders(cfg.CustomProviders) — a custom provider that
// survived built-in-collision / duplicate filtering and is safe to spawn.
// Callers deciding whether a provider string is spawnable use this alongside
// the built-in checks (see internal/hub's validSpawnProvider).
func (cfg *Config) IsCustomProviderID(id string) bool {
	if cfg == nil {
		return false
	}
	id = NormalizeSubscriptionID(id)
	if id == "" {
		return false
	}
	for _, p := range EffectiveCustomProviders(cfg.CustomProviders) {
		if p.ID == id {
			return true
		}
	}
	return false
}

// EffectiveCustomProviders returns the entries safe to offer as spawn
// options: well-formed ids that don't collide with a built-in provider or
// with each other, and that carry a command. Callers that feed the spawn UI
// or spawn validation use this instead of reading cfg.CustomProviders
// directly.
func EffectiveCustomProviders(raw CustomProviders) CustomProviders {
	kept, _, _ := filterCustomProviders(raw)
	return kept
}

// filterCustomProviders splits raw into the usable subset and the original
// (un-normalized) ids dropped for being invalid or duplicates, so Warnings()
// can report what was ignored and why.
func filterCustomProviders(raw CustomProviders) (kept CustomProviders, invalidIDs, duplicateIDs []string) {
	seen := make(map[string]bool, len(raw))
	for _, p := range raw {
		id := NormalizeSubscriptionID(p.ID)
		if err := ValidateCustomProviderID(id); err != nil || strings.TrimSpace(p.Command) == "" {
			invalidIDs = append(invalidIDs, p.ID)
			continue
		}
		if seen[id] {
			duplicateIDs = append(duplicateIDs, p.ID)
			continue
		}
		seen[id] = true
		norm := p
		norm.ID = id
		kept = append(kept, norm)
	}
	return kept, invalidIDs, duplicateIDs
}

// customProviderWarnings is Warnings()'s custom_providers contribution — see
// subscriptionWarnings for the same "invalid config must not stop the Hub"
// reasoning.
func (cfg *Config) customProviderWarnings() []string {
	if cfg == nil || len(cfg.CustomProviders) == 0 {
		return nil
	}
	_, invalid, duplicate := filterCustomProviders(cfg.CustomProviders)
	var warnings []string
	for _, id := range invalid {
		warnings = append(warnings, fmt.Sprintf(
			"custom_providers entry %q is invalid (bad id, collides with a built-in provider, or missing command) and will not appear as a spawn option",
			id))
	}
	for _, id := range duplicate {
		warnings = append(warnings, fmt.Sprintf(
			"custom_providers entry %q duplicates another custom provider id and will not appear as a spawn option",
			id))
	}
	return warnings
}

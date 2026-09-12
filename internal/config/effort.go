package config

import (
	"fmt"
	"strings"
)

// effort.go is the one table that names, per provider, whether a launch
// request can carry a reasoning-effort level and which CLI arguments carry it
// (親 plan: docs/local/plan_derived-session-launch.md D8 / 不変条件 7。子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C1).
//
// It follows internal/subscription/usage_source.go: the decision "which
// providers understand effort" lives in exactly one place, and callers ask
// EffortSupportFor instead of matching on the provider string themselves. A
// new `case "claude":` switch for effort must not appear in internal/hub or
// internal/wrapper — adding a provider means adding a row here.
//
// This file also holds the two other launch-request enums introduced with
// effort (execution mode and permission preset), because they share the same
// property: a value the request may carry, validated in one place, with the
// values this build actually accepts recorded next to the values the schema
// defines. Values that later phases of the parent plan will enable are listed
// but rejected today — a request that explicitly asks for one gets an error
// rather than being silently downgraded (親 plan D2).

// EffortSupport is one row of the effort dispatch table.
type EffortSupport struct {
	// Provider is the many-ai-cli provider id ("claude", "codex", ...).
	Provider string
	// Levels are the effort values offered in the UI for Provider, in the
	// order they should be shown. When AcceptsAnyLevel is false this is also
	// the exhaustive set of accepted values.
	Levels []string
	// AcceptsAnyLevel reports that Provider takes a free-form level, so
	// validation only checks the value's shape, not its membership in
	// Levels. opencode is the case today: `--variant` names come from the
	// model definition rather than from a fixed ladder, so Levels there is a
	// UI suggestion list, not the provider's accepted set.
	AcceptsAnyLevel bool
	// Args returns the CLI arguments that carry level for Provider. It is
	// only called with a level that already passed ValidateEffort.
	Args func(level string) []string
}

// effortSupports is the table. Providers with no row have no mapping at all:
// a request that names one of them with a non-empty effort is rejected
// (EffortSupportFor returns false), and neither the launch form nor the spawn
// confirmation dialog offers the field for them. copilot (config-file
// `effortLevel` only, no flag), grok and cursor-agent (no effort concept) and
// command-code (unverified) are deliberately absent — measured against each
// CLI's --help on 2026-09-12, recorded in the 子 plan's 前提 section. Custom
// providers are never in this table by construction: many-ai-cli does not
// know an arbitrary CLI's flags, which is the same reason it withholds
// --model from them (resolveSpawnModel).
var effortSupports = []EffortSupport{
	{
		Provider: "claude",
		Levels:   []string{"low", "medium", "high", "xhigh", "max"},
		Args: func(level string) []string {
			return []string{"--effort", level}
		},
	},
	{
		Provider: "codex",
		Levels:   []string{"minimal", "low", "medium", "high", "xhigh", "max"},
		Args: func(level string) []string {
			// codex --help documents -c values as quoted strings
			// (`-c model="o3"`), so the quotes are part of the value and are
			// passed through literally rather than being shell quoting.
			return []string{"-c", fmt.Sprintf("model_reasoning_effort=%q", level)}
		},
	},
	{
		Provider: "opencode",
		// Suggestions only — see AcceptsAnyLevel. These three are the ladder
		// the UI offers when nothing better is known; opencode itself takes
		// whatever variant name the selected model defines.
		Levels:          []string{"low", "medium", "high"},
		AcceptsAnyLevel: true,
		Args: func(level string) []string {
			return []string{"--variant", level}
		},
	},
}

// MaxEffortLevelLen bounds an effort value. Levels are short words, never
// paths or sentences; the bound exists so a free-form provider (opencode)
// cannot be used to push an arbitrary blob into a command line.
const MaxEffortLevelLen = 32

// EffortSupportFor returns provider's row and whether one exists. A provider
// without a row has no effort mapping: requests must leave effort empty for
// it.
func EffortSupportFor(provider string) (EffortSupport, bool) {
	for _, row := range effortSupports {
		if row.Provider == provider {
			return row, true
		}
	}
	return EffortSupport{}, false
}

// EffortProviders returns the provider ids that have an effort mapping, in
// table order.
func EffortProviders() []string {
	out := make([]string, 0, len(effortSupports))
	for _, row := range effortSupports {
		out = append(out, row.Provider)
	}
	return out
}

// EffortLevelsFor returns the levels to offer for provider, or nil when
// provider has no mapping.
func EffortLevelsFor(provider string) []string {
	row, ok := EffortSupportFor(provider)
	if !ok {
		return nil
	}
	return append([]string(nil), row.Levels...)
}

// validEffortLevelShape reports whether level is shaped like an effort level:
// a short run of letters, digits, '-', '_' or '.'. It is the shape check for
// free-form providers, and a second line of defence for the fixed ones — an
// effort value reaches a command line, so it must never be able to look like
// a flag or carry a shell metacharacter.
func validEffortLevelShape(level string) bool {
	if level == "" || len(level) > MaxEffortLevelLen {
		return false
	}
	if level[0] == '-' {
		return false
	}
	for i := 0; i < len(level); i++ {
		c := level[i]
		switch {
		case c >= 'a' && c <= 'z':
		case c >= 'A' && c <= 'Z':
		case c >= '0' && c <= '9':
		case c == '-' || c == '_' || c == '.':
		default:
			return false
		}
	}
	return true
}

// ValidateEffort checks an effort level for provider. An empty level is
// always valid and means "not specified" — every launch path that omits
// effort keeps behaving exactly as it did before this field existed.
func ValidateEffort(provider, level string) error {
	if level == "" {
		return nil
	}
	row, ok := EffortSupportFor(provider)
	if !ok {
		return fmt.Errorf("provider %q does not support effort", provider)
	}
	if !validEffortLevelShape(level) {
		return fmt.Errorf("invalid effort value")
	}
	if row.AcceptsAnyLevel {
		return nil
	}
	for _, known := range row.Levels {
		if known == level {
			return nil
		}
	}
	return fmt.Errorf("invalid effort %q for provider %q", level, provider)
}

// EffortArgs returns the CLI arguments carrying level for provider, or nil
// when provider has no mapping or level is empty. Callers use the nil result
// as "add nothing", so a provider without a mapping changes no command line.
func EffortArgs(provider, level string) []string {
	if level == "" {
		return nil
	}
	row, ok := EffortSupportFor(provider)
	if !ok || row.Args == nil {
		return nil
	}
	return row.Args(level)
}

// Execution modes. Auto lets the Hub decide (親 plan D2: unattended children
// prefer headless where the provider supports it, anything a human opens is
// interactive); Interactive is today's only behaviour. Headless is part of
// the schema from the start so that a caller asking for it gets a clear
// rejection instead of silently receiving an interactive session.
const (
	ExecutionModeUnset       = ""
	ExecutionModeAuto        = "auto"
	ExecutionModeInteractive = "interactive"
	ExecutionModeHeadless    = "headless"
)

// Permission presets. Attended leaves approval settings alone so approvals
// reach the Hub's approval panel; Bounded is the narrowed unattended tier (no
// prompts, but only the allowlisted actions); Full is today's
// orchestration-child default. What each one means per provider lives in
// internal/hub/child_permission.go.
const (
	PermissionPresetUnset    = ""
	PermissionPresetAttended = "attended"
	PermissionPresetBounded  = "bounded"
	PermissionPresetFull     = "full"
)

// knownExecutionModes / knownPermissionPresets are the schema. The
// "available" subsets below are what this build accepts; the difference is
// what later phases of the parent plan enable.
var (
	knownExecutionModes = []string{ExecutionModeAuto, ExecutionModeInteractive, ExecutionModeHeadless}
	// All three modes are available: headless joined them with the headless
	// definition table (子 plan plan_child_execution_modes_headless.md 内部 C1).
	// A provider with no definition does not make the mode unavailable — the
	// request is accepted here and resolved per provider by
	// ResolveExecutionMode, which is where "this provider cannot do headless"
	// becomes an error (親 plan D2).
	availableExecutionModes = []string{ExecutionModeAuto, ExecutionModeInteractive, ExecutionModeHeadless}

	knownPermissionPresets = []string{PermissionPresetAttended, PermissionPresetBounded, PermissionPresetFull}
	// All three presets are available: bounded joined them with the tier table
	// (子 plan plan_derived-session-launch_c2_permission-tiers.md 内部 C3). A
	// provider with no bounded tier does not make the preset unavailable — the
	// request is accepted and falls through to full, and the confirmation
	// dialog says so (親 plan 不変条件 5).
	availablePermissionPresets = []string{PermissionPresetAttended, PermissionPresetBounded, PermissionPresetFull}
)

// NormalizeExecutionMode trims surrounding whitespace. It deliberately does
// not map unknown values onto a default: an unrecognised mode is an error,
// not something to guess at.
func NormalizeExecutionMode(mode string) string { return strings.TrimSpace(mode) }

// NormalizePermissionPreset trims surrounding whitespace.
func NormalizePermissionPreset(preset string) string { return strings.TrimSpace(preset) }

// ValidateExecutionMode accepts the modes this build can honour. An empty
// mode means "not specified" and behaves exactly as before the field
// existed.
//
// This is the schema check only. Whether *this provider* can honour headless
// is decided later, per launch, by ResolveExecutionMode — which turns an
// explicit headless on a provider without a definition into an error rather
// than downgrading it to interactive (親 plan D2 requires an explicit request
// never to be silently changed).
func ValidateExecutionMode(mode string) error {
	if mode == ExecutionModeUnset {
		return nil
	}
	for _, known := range availableExecutionModes {
		if known == mode {
			return nil
		}
	}
	return fmt.Errorf("invalid execution_mode %q", mode)
}

// ValidatePermissionPreset accepts the presets this build can honour.
func ValidatePermissionPreset(preset string) error {
	if preset == PermissionPresetUnset {
		return nil
	}
	for _, known := range availablePermissionPresets {
		if known == preset {
			return nil
		}
	}
	return fmt.Errorf("invalid permission_preset %q", preset)
}

// KnownExecutionModes / AvailableExecutionModes / KnownPermissionPresets /
// AvailablePermissionPresets expose the two sets as copies so callers (the
// launch form's level list, the confirmation dialog's disabled options) never
// mutate the tables.
func KnownExecutionModes() []string { return append([]string(nil), knownExecutionModes...) }

// AvailableExecutionModes returns the execution modes this build accepts.
func AvailableExecutionModes() []string { return append([]string(nil), availableExecutionModes...) }

// KnownPermissionPresets returns every preset in the schema, including the
// ones this build rejects.
func KnownPermissionPresets() []string { return append([]string(nil), knownPermissionPresets...) }

// AvailablePermissionPresets returns the presets this build accepts.
func AvailablePermissionPresets() []string {
	return append([]string(nil), availablePermissionPresets...)
}

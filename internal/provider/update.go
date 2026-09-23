package provider

import "fmt"

const (
	defaultVersionArg       = "--version"
	defaultUpdateTimeoutSec = 300
	maxUpdateTimeoutSec     = 3600
)

// ResolveVersionArgs returns the argv (excluding the executable) used to
// check a provider's version. Definitions that omit version_args fall back
// to --version rather than baking the default into every manifest.
func ResolveVersionArgs(update *UpdateDefinition) []string {
	if update == nil || len(update.VersionArgs) == 0 {
		return []string{defaultVersionArg}
	}
	return append([]string(nil), update.VersionArgs...)
}

// ResolveUpdateTimeoutSeconds returns the update timeout, defaulting to
// defaultUpdateTimeoutSec when unset.
func ResolveUpdateTimeoutSeconds(update *UpdateDefinition) int {
	if update == nil || update.TimeoutSeconds <= 0 {
		return defaultUpdateTimeoutSec
	}
	return update.TimeoutSeconds
}

// UpdateEnabled reports whether the update button should be offered for this
// definition. An explicit enabled flag always wins; otherwise the presence
// of update.args is the default (a manifest with no update args obviously
// has nothing to run).
func UpdateEnabled(update *UpdateDefinition) bool {
	if update == nil {
		return false
	}
	if update.Enabled != nil {
		return *update.Enabled
	}
	return len(update.Args) > 0
}

// ResolveUpdateArgv computes the argv (executable followed by args) used to
// update the CLI, given the executable name that launch actually selected
// (e.g. "copilot" or its "gh" fallback candidate). Callers do not assemble
// this argv themselves so every entry point (version check, update job,
// settings UI preview) agrees on the same rule:
//
//   - update.executable, when set, is always used regardless of which
//     candidate launch selected (the manifest author pinned it on purpose).
//   - otherwise update.args only applies to the primary launch executable
//     (launch.executable, or executable_candidates[0] when unset). A provider
//     started via a secondary candidate (copilot's "gh" fallback) has no
//     configured update method unless update.executable says otherwise.
//
// When no argv can be resolved, argv is nil and unavailableReason explains
// why (update missing, args empty, or executable mismatch).
func ResolveUpdateArgv(definition Definition, selectedExecutable string) (argv []string, unavailableReason string) {
	update := definition.Update
	if update == nil || len(update.Args) == 0 {
		return nil, "update method is not configured"
	}
	executable := update.Executable
	if executable == "" {
		primary := primaryLaunchExecutable(definition.Launch)
		if primary == "" || selectedExecutable != primary {
			return nil, fmt.Sprintf("update method is not configured for executable %q", selectedExecutable)
		}
		executable = selectedExecutable
	}
	out := make([]string, 0, 1+len(update.Args))
	out = append(out, executable)
	out = append(out, update.Args...)
	return out, ""
}

func primaryLaunchExecutable(launch *LaunchDefinition) string {
	if launch == nil {
		return ""
	}
	if launch.Executable != "" {
		return launch.Executable
	}
	if len(launch.ExecutableCandidates) > 0 {
		return launch.ExecutableCandidates[0]
	}
	return ""
}

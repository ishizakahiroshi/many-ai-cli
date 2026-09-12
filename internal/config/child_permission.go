package config

import (
	"fmt"
	"sort"
	"strings"
)

// child_permission.go holds the configurable half of the child permission
// tiers (親 plan: docs/local/plan_derived-session-launch.md D3 / 不変条件 7.
// 子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C1).
//
// The tiers themselves — which flags each provider starts with for attended /
// bounded / full — live in exactly one table, internal/hub/child_permission.go.
// What lives here is the part a user is expected to edit:
//
//   - orchestration.child_permission_default: which tier an unattended child
//     gets when the request names none.
//   - orchestration.bounded_allowed_tools: the per-provider allowlist the
//     bounded tier hands to the CLI, with a built-in default per provider.
//
// The built-in allowlists are deliberately narrow. A bounded child is meant to
// be able to read, edit and verify inside its worktree and to record its work
// with git, and nothing else: `git push`, `git reset`, `git clean` and `rm` are
// absent on purpose, so an unattended child that reaches for one is blocked and
// reports it rather than doing it. Widening them is a user decision, made in
// config.yaml, not a default.

// PermissionModeBounded is the internal permission-mode marker the bounded tier
// uses for the providers whose narrowing is not a flag of its own: copilot
// (an --allow-tool list) and opencode (permission rules in opencode.json). It
// travels Hub → `many-ai-cli wrap` and is translated there by that provider's
// existing permission-args helper; the provider CLI never receives the word
// "bounded". It is deliberately absent from /api/spawn's accepted
// permission_mode values — it is a tier, not something a caller names directly.
const PermissionModeBounded = "bounded"

// MaxAllowedToolValueLen bounds one allowlist entry. Entries are short tool
// names or command patterns, never paths or prose; the bound exists because the
// value reaches a command line.
const MaxAllowedToolValueLen = 120

// defaultBoundedAllowedTools is the built-in allowlist per provider, in the
// syntax each CLI documents for itself:
//
//   - claude: `--allowedTools` takes a "Comma or space-separated list of tool
//     names to allow (e.g. "Bash(git *) Edit")" (claude --help, measured
//     2026-09-12). Bash command patterns are listed both bare and with a
//     trailing wildcard so a plain `git status` is covered as well as
//     `git status --short`.
//   - copilot: `--allow-tool` takes values such as `shell(git:*)` and `write`
//     (copilot --help examples, measured 2026-09-12), where `:*` is the prefix
//     form.
//
// Providers absent from this map have no bounded tier at all (grok,
// cursor-agent, command-code) or need no allowlist for it (codex bounds itself
// with --sandbox workspace-write, opencode with its own permission rules).
var defaultBoundedAllowedTools = map[string][]string{
	"claude": {
		"Read", "Glob", "Grep", "Edit", "Write", "NotebookEdit", "TodoWrite",
		"Bash(git status)", "Bash(git status *)",
		"Bash(git diff)", "Bash(git diff *)",
		"Bash(git log)", "Bash(git log *)",
		"Bash(git add *)",
		"Bash(git commit *)",
		"Bash(go test *)",
		"Bash(go vet *)",
		"Bash(gofmt *)",
		"Bash(bun run check)", "Bash(bun run check *)",
	},
	"copilot": {
		"write",
		"shell(git status:*)",
		"shell(git diff:*)",
		"shell(git log:*)",
		"shell(git add:*)",
		"shell(git commit:*)",
		"shell(go test:*)",
		"shell(go vet:*)",
		"shell(gofmt:*)",
		"shell(bun run check:*)",
	},
}

// ValidAllowedToolValue reports whether one allowlist entry is shaped like a
// tool name or command pattern. It is an allowlist of characters rather than a
// denylist of dangerous ones: the value is handed to a child process argument
// list, and on Windows some launches go through a cmd.exe shim, so anything
// that could read as shell syntax must never get through. A comma is rejected
// too because the wrapper flag that carries these values is comma-separated.
func ValidAllowedToolValue(value string) bool {
	if value == "" || len(value) > MaxAllowedToolValueLen {
		return false
	}
	if value[0] == '-' || value[0] == ' ' {
		return false
	}
	for i := 0; i < len(value); i++ {
		c := value[i]
		switch {
		case c >= 'a' && c <= 'z':
		case c >= 'A' && c <= 'Z':
		case c >= '0' && c <= '9':
		case strings.IndexByte(" ()*.:/_-", c) >= 0:
		default:
			return false
		}
	}
	return true
}

// BoundedAllowedToolsFor returns the allowlist the bounded tier should hand to
// provider: the user's orchestration.bounded_allowed_tools entry when it has
// one, otherwise the built-in default. An entry that fails
// ValidAllowedToolValue is dropped here rather than passed on; Warnings() names
// the dropped values so the user is not left wondering why one had no effect.
//
// A provider with no entry and no built-in default returns nil, which callers
// read as "add no allowlist flag at all".
func (o OrchestrationConfig) BoundedAllowedToolsFor(provider string) []string {
	provider = strings.TrimSpace(provider)
	if provider == "" {
		return nil
	}
	source, ok := o.BoundedAllowedTools[provider]
	if !ok {
		source = defaultBoundedAllowedTools[provider]
	}
	out := make([]string, 0, len(source))
	for _, raw := range source {
		value := strings.TrimSpace(raw)
		if ValidAllowedToolValue(value) {
			out = append(out, value)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

// ChildPermissionDefaultTier returns the tier an unattended child gets when the
// launch request names no permission_preset. Unset, and anything this build
// does not recognise, means full — today's behaviour
// (applyChildApprovalDefaults). Changing the built-in default is a separate
// decision that waits on a user's own relay run (子 plan C4).
func (o OrchestrationConfig) ChildPermissionDefaultTier() string {
	switch strings.TrimSpace(o.ChildPermissionDefault) {
	case PermissionPresetAttended:
		return PermissionPresetAttended
	case PermissionPresetBounded:
		return PermissionPresetBounded
	default:
		return PermissionPresetFull
	}
}

// childPermissionWarnings reports settings that were accepted into config.yaml
// but cannot take effect, following the same rule as the other optional
// features: a bad value for an optional setting must not stop the Hub from
// starting, it must say why the setting did nothing.
func (cfg *Config) childPermissionWarnings() []string {
	if cfg == nil {
		return nil
	}
	var warnings []string
	if raw := strings.TrimSpace(cfg.Orchestration.ChildPermissionDefault); raw != "" {
		switch raw {
		case PermissionPresetAttended, PermissionPresetBounded, PermissionPresetFull:
		default:
			warnings = append(warnings, fmt.Sprintf(
				"orchestration.child_permission_default %q is not one of attended/bounded/full; unattended children keep the built-in default (%s).",
				raw, PermissionPresetFull))
		}
	}
	providers := make([]string, 0, len(cfg.Orchestration.BoundedAllowedTools))
	for provider := range cfg.Orchestration.BoundedAllowedTools {
		providers = append(providers, provider)
	}
	sort.Strings(providers)
	for _, provider := range providers {
		var dropped []string
		for _, raw := range cfg.Orchestration.BoundedAllowedTools[provider] {
			if !ValidAllowedToolValue(strings.TrimSpace(raw)) {
				dropped = append(dropped, raw)
			}
		}
		if len(dropped) > 0 {
			warnings = append(warnings, fmt.Sprintf(
				"orchestration.bounded_allowed_tools[%s] has %d entry/entries that are not a tool name or command pattern and were ignored: %s",
				provider, len(dropped), strings.Join(dropped, " | ")))
		}
	}
	return warnings
}

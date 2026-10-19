package config

import (
	"fmt"
	"sort"
	"strings"
)

// headless.go is the one table that says, per provider, how to start that CLI
// in its own non-interactive mode, plus the one pure function that decides
// which execution mode a launch actually runs in
// (親 plan: docs/local/plan_derived-session-launch.md D2 / D9 / 不変条件 7.
// 子 plan: docs/local/plan_child_execution_modes_headless.md 内部 C1).
//
// It follows internal/config/effort.go and internal/subscription/usage_source.go:
// the provider difference is data, and only the interpretation is code. A
// `case "claude":` for headless must not appear in internal/headless,
// internal/hub or internal/wrapper — adding a provider means adding a row here
// (or, for a CLI many-ai-cli does not ship, a `headless:` block under that
// entry in config.yaml's `custom_providers:`).
//
// A row carries three things and nothing else:
//
//	Args       the flags that put this CLI into its print / exec / run mode
//	Format     the name of the stdout format, which selects a parser
//	PromptVia  where the prompt goes in (stdin, or the first positional arg)
//
// Everything else a headless launch needs — model, effort, permission flags,
// the Claude --session-id — is built by the same code the interactive launch
// uses and appended after Args, so the two modes cannot drift apart
// (親 plan からの差分 5).

// Headless stdout formats. The name selects a parser in internal/headless;
// "text" is the generic one (take the bytes as they come, let the exit code
// decide the outcome), which is what lets a print-mode CLI become an
// unattended child with a definition alone and no new Go code.
const (
	HeadlessFormatText             = "text"
	HeadlessFormatClaudeStreamJSON = "claude-stream-json"
)

// Where the prompt is handed to the CLI. Stdin is preferred wherever the CLI
// reads one: a positional prompt has to sit somewhere in an argument list that
// already carries variadic flags (Claude's --allowedTools takes a list), and
// "somewhere" is exactly the kind of ordering rule that breaks silently.
const (
	HeadlessPromptViaStdin = "stdin"
	HeadlessPromptViaArg   = "arg"
)

// MaxHeadlessArgs / MaxHeadlessArgLen bound one definition. These are flags
// typed into a config file, not data: a definition that needs more than this
// is a sign the value is being used as a payload.
const (
	MaxHeadlessArgs   = 32
	MaxHeadlessArgLen = 200
)

// HeadlessDef is one provider's headless definition.
type HeadlessDef struct {
	// Args are the flags that select the CLI's non-interactive mode, in the
	// order they must appear. They are placed directly after the executable,
	// before the arguments the launch itself contributes (model / effort /
	// permission). They are passed to exec.Command as separate argv entries —
	// never through a shell — so they must not be pre-quoted.
	Args []string `yaml:"args,omitempty" json:"args,omitempty"`
	// Format names the stdout format, and therefore the parser. Empty means
	// the entry has no headless support at all.
	Format string `yaml:"format,omitempty" json:"format,omitempty"`
	// PromptVia is "stdin" (default) or "arg". With "arg" the prompt is the
	// **first** positional argument, placed immediately after Args, so a
	// variadic flag later in the line cannot swallow it.
	PromptVia string `yaml:"prompt_via,omitempty" json:"prompt_via,omitempty"`
}

// builtinHeadlessDefs is the table for the providers many-ai-cli ships.
//
// Every row was measured against that CLI's own `--help` on this machine on
// 2026-09-12, and the flags it names are only the ones that select print mode
// and its output format. Model, effort and permission flags are never written
// here: they are appended by the same code the interactive launch uses, so the
// two modes cannot drift (親 plan からの差分 5). A provider with no row is simply
// not headless-capable, which `auto` handles by staying interactive and an
// explicit `headless` handles by failing loudly (ResolveExecutionMode).
//
// Format is "text" everywhere but claude — deliberately. "text" means "show the
// CLI's own output and let the exit code decide the outcome", which is exactly
// what the session card shows and what the relay acts on. A structured parser is
// only worth writing against a documented line shape, and nothing consumes the
// extra structure today (the event viewer is not built: 子 plan 内部 C6).
//
// A note on where the prompt goes. stdin is preferred, but only claude documents
// reading it there (--input-format). For the rest, `--help` says the prompt is a
// flag value or a positional, so PromptVia is "arg" and BuildArgv places it
// immediately after these flags — which is why a row whose print flag *takes*
// the prompt (grok -p, copilot -p, command-code -p) ends with that flag.
//
// **codex is deliberately absent.** `codex exec --json` is the right entry
// point, but `codex exec` does not accept `--ask-for-approval`
// (measured: "error: unexpected argument '--ask-for-approval' found"), and the
// permission table gives every codex child `--ask-for-approval never` at both
// the bounded and full tier (internal/hub/child_permission.go). A row here would
// therefore build a command line that always fails to parse. Adding codex means
// first giving its permission tiers an exec-mode form; that is a change to the
// permission table, not to this one (子 plan 内部 C5 の実装メモ).
//
// The measurements, per row:
//
//	claude         -p, --print              "Print response and exit (useful for pipes)"
//	               --output-format <format> "only works with --print": text | json | stream-json
//	               --verbose                so the stream carries tool/turn events,
//	                                        not just the final text
//	               --input-format <format>  "only works with --print" — the statement
//	                                        that print mode reads stdin
//	grok           -p, --single <PROMPT>    "Single-turn prompt. Prints the response
//	                                        to stdout and exits" (takes the prompt)
//	               --output-format          plain | json | streaming-json |
//	                                        streaming-messages-json, default plain
//	cursor-agent   -p, --print              "Print responses to console (for scripts
//	                                        or non-interactive use)" — a boolean; the
//	                                        prompt is the positional [prompt...]
//	               --output-format <format> "only works with --print": text | json |
//	                                        stream-json, default text
//	opencode       run [message..]          "run opencode with a message"
//	               --format                 default (formatted) | json, default default
//	copilot        -p, --prompt <text>      "Execute a prompt in non-interactive mode
//	                                        (exits after completion)" (takes the prompt)
//	               --output-format <format> text | json, default text
//	               -s, --silent             "Output only the agent response (no stats),
//	                                        useful for scripting with -p"
//	command-code   -p, --print [query]      "Run in non-interactive mode, output
//	                                        response and exit" (takes the prompt)
//	               --output-format          text (default) | json
var builtinHeadlessDefs = map[string]HeadlessDef{
	"claude": {
		Args:      []string{"-p", "--output-format", "stream-json", "--verbose"},
		Format:    HeadlessFormatClaudeStreamJSON,
		PromptVia: HeadlessPromptViaStdin,
	},
	"grok": {
		Args:      []string{"--output-format", "plain", "-p"},
		Format:    HeadlessFormatText,
		PromptVia: HeadlessPromptViaArg,
	},
	"cursor-agent": {
		Args:      []string{"-p", "--output-format", "text"},
		Format:    HeadlessFormatText,
		PromptVia: HeadlessPromptViaArg,
	},
	"opencode": {
		Args:      []string{"run", "--format", "default"},
		Format:    HeadlessFormatText,
		PromptVia: HeadlessPromptViaArg,
	},
	"copilot": {
		Args:      []string{"--output-format", "text", "-s", "-p"},
		Format:    HeadlessFormatText,
		PromptVia: HeadlessPromptViaArg,
	},
	"command-code": {
		Args:      []string{"--output-format", "text", "-p"},
		Format:    HeadlessFormatText,
		PromptVia: HeadlessPromptViaArg,
	},
}

// KnownHeadlessFormats returns every format name a definition may carry. The
// parsers live in internal/headless; that package's test asserts it has one for
// each name here, so a name cannot be accepted into config.yaml with nothing
// behind it.
func KnownHeadlessFormats() []string {
	return []string{HeadlessFormatText, HeadlessFormatClaudeStreamJSON}
}

// NormalizeHeadlessPromptVia maps an empty or unrecognised value onto stdin.
// It is only reached for values ValidateHeadlessDef already accepted.
func NormalizeHeadlessPromptVia(via string) string {
	if strings.TrimSpace(via) == HeadlessPromptViaArg {
		return HeadlessPromptViaArg
	}
	return HeadlessPromptViaStdin
}

// ValidateHeadlessDef checks a definition that came from config.yaml. A broken
// one is reported (customProviderWarnings) and ignored — it never stops the Hub
// from starting, and it never reaches a command line.
func ValidateHeadlessDef(def HeadlessDef) error {
	format := strings.TrimSpace(def.Format)
	if format == "" {
		return fmt.Errorf("headless.format is required")
	}
	known := false
	for _, name := range KnownHeadlessFormats() {
		if name == format {
			known = true
			break
		}
	}
	if !known {
		return fmt.Errorf("headless.format %q is not one of %s", format, strings.Join(KnownHeadlessFormats(), ", "))
	}
	switch strings.TrimSpace(def.PromptVia) {
	case "", HeadlessPromptViaStdin, HeadlessPromptViaArg:
	default:
		return fmt.Errorf("headless.prompt_via %q is not %s or %s", def.PromptVia, HeadlessPromptViaStdin, HeadlessPromptViaArg)
	}
	if len(def.Args) > MaxHeadlessArgs {
		return fmt.Errorf("headless.args has more than %d entries", MaxHeadlessArgs)
	}
	for _, arg := range def.Args {
		if err := validHeadlessArg(arg); err != nil {
			return err
		}
	}
	return nil
}

// validHeadlessArg rejects an argument that could not be a flag or a flag
// value. The check is deliberately about shape, not about a denylist of
// dangerous characters: these strings become entries in a child process's
// argument list, and on Windows some launches pass through a cmd.exe shim, so
// anything carrying a control character or a newline is refused outright.
func validHeadlessArg(arg string) error {
	if arg == "" {
		return fmt.Errorf("headless.args has an empty entry")
	}
	if len(arg) > MaxHeadlessArgLen {
		return fmt.Errorf("headless.args entry is longer than %d characters", MaxHeadlessArgLen)
	}
	for i := 0; i < len(arg); i++ {
		if arg[i] < 0x20 || arg[i] == 0x7f {
			return fmt.Errorf("headless.args entry contains a control character")
		}
	}
	return nil
}

// HeadlessDefFor returns provider's definition and whether one exists. cfg may
// be nil, in which case only the built-in table is consulted — a caller that
// has no config snapshot at hand (a preview, a test) still sees the same
// answer for every provider many-ai-cli ships.
//
// The returned definition is a copy: the table and the user's config are never
// handed out by reference, so a caller cannot mutate what the next launch sees.
func HeadlessDefFor(provider string, cfg *Config) (HeadlessDef, bool) {
	provider = NormalizeSubscriptionID(provider)
	if provider == "" {
		return HeadlessDef{}, false
	}
	if def, ok := builtinHeadlessDefs[provider]; ok {
		return cloneHeadlessDef(def), true
	}
	if cfg == nil {
		return HeadlessDef{}, false
	}
	for _, p := range EffectiveCustomProviders(cfg.CustomProviders) {
		if p.ID != provider || p.Headless == nil {
			continue
		}
		if err := ValidateHeadlessDef(*p.Headless); err != nil {
			return HeadlessDef{}, false
		}
		return cloneHeadlessDef(*p.Headless), true
	}
	return HeadlessDef{}, false
}

// HeadlessProviders returns every provider that can be launched
// non-interactively — the built-in table plus any `custom_providers:` entry with
// a valid `headless:` block — sorted, so the list is stable between calls.
//
// It exists for one job: telling the screen, before anyone presses a button,
// that a CLI has no headless definition. **It is not where the decision is
// made.** Every launch still resolves its own mode (ResolveExecutionMode), and
// that resolution is what refuses an impossible request; this list only lets the
// refusal be predicted out loud instead of arriving as an error afterwards.
func HeadlessProviders(cfg *Config) []string {
	seen := make(map[string]struct{}, len(builtinHeadlessDefs))
	out := make([]string, 0, len(builtinHeadlessDefs))
	for provider := range builtinHeadlessDefs {
		seen[provider] = struct{}{}
		out = append(out, provider)
	}
	if cfg != nil {
		for _, p := range EffectiveCustomProviders(cfg.CustomProviders) {
			if p.Headless == nil || p.ID == "" {
				continue
			}
			if _, exists := seen[p.ID]; exists {
				continue
			}
			if err := ValidateHeadlessDef(*p.Headless); err != nil {
				continue
			}
			seen[p.ID] = struct{}{}
			out = append(out, p.ID)
		}
	}
	sort.Strings(out)
	return out
}

func cloneHeadlessDef(def HeadlessDef) HeadlessDef {
	out := def
	out.Format = strings.TrimSpace(def.Format)
	out.PromptVia = NormalizeHeadlessPromptVia(def.PromptVia)
	out.Args = append([]string(nil), def.Args...)
	return out
}

// Launch origins. The empty string is an AI conductor's `orchestrate spawn` and
// every relay child — nobody is watching those — and "ui" is a human pressing a
// button on the screen. internal/hub's launchOriginConductor / launchOriginUI
// are these same two values; they live here as well because the execution-mode
// resolution below is the one place that has to know the difference and it must
// stay a pure function.
const (
	LaunchOriginConductor = ""
	LaunchOriginUI        = "ui"
)

// ResolveExecutionMode decides the mode a launch actually runs in.
//
//	requested    what the caller asked for ("" / auto / interactive / headless)
//	capable      whether this provider has a headless definition
//	origin       LaunchOriginConductor or LaunchOriginUI
//	unattended   the caller already knows nobody is watching (a relay worker),
//	             regardless of which origin started the run
//
// The rules, in the order they matter:
//
//   - An omitted mode stays omitted. Every call that predates this field keeps
//     the exact request it sent, byte for byte (親 plan 不変条件 1), and an
//     omitted mode means interactive everywhere it is read.
//   - An explicit headless on a provider that cannot do it is an **error**, not
//     a downgrade (親 plan D2). "I asked for headless and it quietly opened an
//     interactive session that then sat waiting" is the accident this prevents.
//   - auto picks headless only when nobody is watching and the provider can do
//     it. Anything a human opened stays interactive, because the value of an
//     interactive session is that the human can type in it.
func ResolveExecutionMode(requested string, capable bool, origin string, unattended bool) (string, error) {
	switch NormalizeExecutionMode(requested) {
	case ExecutionModeUnset:
		return ExecutionModeUnset, nil
	case ExecutionModeInteractive:
		return ExecutionModeInteractive, nil
	case ExecutionModeHeadless:
		if !capable {
			return "", fmt.Errorf("execution_mode headless is not supported for this provider")
		}
		return ExecutionModeHeadless, nil
	case ExecutionModeAuto:
		if capable && (unattended || strings.TrimSpace(origin) != LaunchOriginUI) {
			return ExecutionModeHeadless, nil
		}
		return ExecutionModeInteractive, nil
	default:
		return "", fmt.Errorf("invalid execution_mode %q", requested)
	}
}

// IsHeadlessExecutionMode reports whether a resolved mode means "no PTY". It
// exists so callers do not compare against the constant in a dozen places and
// so an unresolved value ("auto", which must never reach a launch) reads as
// false rather than as a silent headless.
func IsHeadlessExecutionMode(mode string) bool {
	return NormalizeExecutionMode(mode) == ExecutionModeHeadless
}

// ChildExecutionModeDefault returns the execution mode an orchestration child
// gets when its request names none. Unset — and anything this build does not
// recognise — means the request stays unset, which is today's behaviour
// (interactive). It is deliberately separate from the relay's own default:
// relay children are a different kind of worker and get their own setting when
// the relay side lands (子 plan 内部 C4).
func (o OrchestrationConfig) ChildExecutionModeDefault() string {
	switch NormalizeExecutionMode(o.ChildExecutionMode) {
	case ExecutionModeAuto:
		return ExecutionModeAuto
	case ExecutionModeInteractive:
		return ExecutionModeInteractive
	case ExecutionModeHeadless:
		return ExecutionModeHeadless
	default:
		return ExecutionModeUnset
	}
}

// RelayExecutionModeDefault returns the execution mode a relay role gets when
// its own assignment names none. It is deliberately separate from
// ChildExecutionModeDefault: a relay worker is a different kind of child. An
// interactive relay child stays alive and is handed each next instruction
// through its input box, while a headless one runs **one process per
// instruction** and its exit is what tells the relay that instruction is over
// (子 plan 内部 C4). Somebody who wants the first shape for `orchestrate spawn`
// and the second for relays must be able to say so.
//
// Unset — and anything this build does not recognise — means the roles keep
// whatever they were given, which is today's behaviour (interactive).
func (o OrchestrationConfig) RelayExecutionModeDefault() string {
	switch NormalizeExecutionMode(o.RelayExecutionMode) {
	case ExecutionModeAuto:
		return ExecutionModeAuto
	case ExecutionModeInteractive:
		return ExecutionModeInteractive
	case ExecutionModeHeadless:
		return ExecutionModeHeadless
	default:
		return ExecutionModeUnset
	}
}

// childExecutionModeWarnings reports an orchestration.child_execution_mode or
// orchestration.relay_execution_mode that was accepted into config.yaml but
// cannot take effect — the same rule as every other optional setting: a bad
// value must not stop the Hub, it must say why it did nothing.
func (cfg *Config) childExecutionModeWarnings() []string {
	if cfg == nil {
		return nil
	}
	var out []string
	for _, setting := range []struct {
		key   string
		raw   string
		stays string
	}{
		{"orchestration.child_execution_mode", cfg.Orchestration.ChildExecutionMode, "children"},
		{"orchestration.relay_execution_mode", cfg.Orchestration.RelayExecutionMode, "relay roles"},
	} {
		raw := strings.TrimSpace(setting.raw)
		if raw == "" {
			continue
		}
		switch raw {
		case ExecutionModeAuto, ExecutionModeInteractive, ExecutionModeHeadless:
			continue
		}
		out = append(out, fmt.Sprintf(
			"%s %q is not one of %s; %s keep the built-in default (interactive).",
			setting.key, raw, strings.Join(KnownExecutionModes(), "/"), setting.stays))
	}
	return out
}

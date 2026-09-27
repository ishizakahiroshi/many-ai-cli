// Package headless runs a provider CLI in its own non-interactive mode and
// turns what it prints into the same session output an interactive (PTY)
// session produces.
//
// 正本: docs/local/plan_child_execution_modes_headless.md（親 plan:
// docs/local/plan_derived-session-launch.md C5）.
//
// The division of labour is the point of the package:
//
//	internal/config/headless.go   which flags, which format, where the prompt
//	                              goes — data, one row per provider
//	this package                  how to run a process and how to read each
//	                              named format — code, one parser per format
//	internal/wrapper              which executable, which model / effort /
//	                              permission flags — shared with the PTY path
//
// **There is no provider switch in here.** A provider reaches this package as
// a definition and a format name; if a `case "claude":` ever appears below,
// the split above has been broken (親 plan 不変条件 7).
package headless

import (
	"time"

	"many-ai-cli/internal/config"
)

// Parser turns one line of a provider's stdout into zero or more normalized
// events. It is called once per line, in order, and must not retain the slice
// it is given.
type Parser interface {
	Parse(line []byte) []Event
}

// parsers is the format table: every name config.KnownHeadlessFormats() allows
// has exactly one entry here. internal/headless's own test asserts that, so a
// format name can never be accepted into config.yaml with nothing behind it.
var parsers = map[string]Parser{
	config.HeadlessFormatText:             textParser{},
	config.HeadlessFormatClaudeStreamJSON: claudeStreamJSONParser{},
}

// ParserFor returns the parser for a format name.
func ParserFor(format string) (Parser, bool) {
	p, ok := parsers[format]
	return p, ok
}

// BuildArgv assembles the argument list for a headless launch:
//
//	<def.Args…> [prompt] <launchArgs…>
//
// launchArgs are the arguments the launch itself contributes — model, effort,
// permission mode, the allowlist, Claude's --session-id — built by exactly the
// same code the interactive path uses (親 plan からの差分 5). They come last so
// that adding one cannot disturb the flags that select print mode.
//
// A prompt passed as an argument goes **first**, immediately after the
// definition's own flags, because several of the launch arguments are variadic
// (claude's --allowedTools takes a list) and would swallow a trailing
// positional. Providers that can read the prompt from stdin use that instead
// and never reach this branch.
func BuildArgv(def config.HeadlessDef, launchArgs []string, prompt string) []string {
	argv := make([]string, 0, len(def.Args)+len(launchArgs)+1)
	argv = append(argv, def.Args...)
	if config.NormalizeHeadlessPromptVia(def.PromptVia) == config.HeadlessPromptViaArg {
		argv = append(argv, prompt)
	}
	return append(argv, launchArgs...)
}

// Spec is one headless run, fully resolved: the caller has already decided
// which executable to launch (the wrapper's own resolveCmd, which unwraps npm
// shims and knows the per-provider exceptions) and which environment it runs
// in (the subscription profile's env, inherited like any other session).
type Spec struct {
	// Exe and Argv are passed straight to exec.Command — never through a
	// shell, so nothing in them is quoted or escaped by this package
	// (元設計 26 節).
	Exe  string
	Argv []string
	// Prompt is written to the process's stdin when PromptVia is stdin. When
	// it is "arg" the prompt is already in Argv and stdin is closed
	// immediately, which is what stops a CLI that decides to read stdin
	// anyway from waiting forever.
	Prompt    string
	PromptVia string
	// Format selects the parser. An unknown format is an error at Run time,
	// not a silent fallback: the run would otherwise produce a session whose
	// output looks fine and whose meaning is wrong.
	Format string
	CWD    string
	Env    []string
	// RawLogPrefix is the path prefix for the raw stdout/stderr logs, without
	// a suffix (".stdout.log" / ".stderr.log" are appended). Empty means the
	// raw output is not written to disk at all — which is the default, because
	// raw provider output carries secrets unmasked and this repository only
	// persists it when the user opts in (log.session_enabled).
	RawLogPrefix string
	// Timeout bounds the whole run. Zero means no timeout of this package's
	// own; the Hub's existing child timeout still applies to the session.
	Timeout time.Duration
}

// Result is what a finished run is worth reporting.
type Result struct {
	// State is "completed" or "error", matching the two values the PTY path's
	// classifyExit produces, because both end up in the same session_end.
	State string
	// ExitCode is the process's own exit code. It is the authoritative verdict
	// (元設計 11 節): a provider's own "I failed" event changes what is shown,
	// never this.
	ExitCode int
	// TimedOut / Canceled distinguish the two ways a run can end without the
	// process deciding to.
	TimedOut bool
	Canceled bool
}

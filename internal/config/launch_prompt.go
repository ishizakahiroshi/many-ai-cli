package config

// launch_prompt.go is the one table that says, per provider, whether an
// **interactive** launch can take its first instruction as a launch argument
// instead of having the Hub type it into the CLI's screen — an orchestration
// child's, and that of a conductor or a prompted session started from the
// screen (親 plan: docs/local/plan_child-launch-prompt-and-trust.md 不変条件 1.
// 子 plan: docs/local/plan_child-launch-prompt-and-trust_c4_launch-arg-prompt.md 内部 C1,
// docs/local/plan_child-launch-prompt-and-trust_c5_ui-launched.md 内部 C1).
//
// Why this exists. Typing into a screen the Hub cannot read is what went wrong
// on 2026-09-24 (#53): Claude Code 2.1.281 opened its folder-trust dialog with
// "No, exit" as the default, and the Enter meant to submit the instruction
// answered the dialog instead. A CLI that takes the instruction as an argument
// holds it until its own startup dialogs are done, then runs it — nothing is
// typed, so there is nothing to land on the wrong screen.
//
// It follows headless.go and effort.go: the provider difference is data, and
// only the interpretation is code. A `case "claude":` for this must not appear
// in internal/hub or internal/wrapper — adding a provider means adding a row
// here, after measuring it.
//
// Every row was measured on this machine on 2026-09-24 (the parent plan's
// T1–T5), with Claude Code 2.1.281 and codex v0.156.1: started with a
// two-line instruction as the first positional argument in a folder the CLI
// did not trust yet, each CLI showed its trust dialog, kept the instruction
// through it, and ran it after "Yes" / "Trust and continue". On Windows,
// codex is started through `cmd.exe /c codex.cmd`, which cut the argument at
// its first newline — the wrapper handles that by passing a one-line pointer
// to a file instead (internal/wrapper), not by leaving codex out of this table.
//
// A provider with no row keeps the typed route (the Hub injects the
// instruction after the CLI draws its input box). Custom providers are never
// in this table: nothing about their argument handling has been measured.
// The providers not yet measured are tracked in
// docs/local/pending_launch-prompt-other-providers.md.
var launchPromptViaArg = map[string]bool{
	"claude": true,
	"codex":  true,
}

// LaunchPromptViaArg reports whether an interactive launch of provider takes
// the first instruction as a launch argument. Unknown providers — including
// every custom provider — report false and keep the typed route.
func LaunchPromptViaArg(provider string) bool {
	return launchPromptViaArg[NormalizeSubscriptionID(provider)]
}

package config

import (
	"strings"
	"testing"
)

// The built-in table is the only place a provider's headless flags live. The
// values are measured against `claude --help`; the test pins the shape (print
// mode + a structured stream + a named parser + stdin) rather than restating
// the flags, so a measured change to the flags is a one-line edit in the table.
func TestHeadlessDefForBuiltin(t *testing.T) {
	def, ok := HeadlessDefFor("claude", nil)
	if !ok {
		t.Fatal("HeadlessDefFor(\"claude\") = false, want the built-in definition")
	}
	if err := ValidateHeadlessDef(def); err != nil {
		t.Fatalf("the built-in claude definition does not validate: %v", err)
	}
	joined := strings.Join(def.Args, " ")
	if !strings.Contains(joined, "-p") || !strings.Contains(joined, "stream-json") {
		t.Errorf("claude args = %v, want print mode with a structured stream", def.Args)
	}
	if def.Format != HeadlessFormatClaudeStreamJSON {
		t.Errorf("claude format = %q, want %q", def.Format, HeadlessFormatClaudeStreamJSON)
	}
	if def.PromptVia != HeadlessPromptViaStdin {
		t.Errorf("claude prompt_via = %q, want %q", def.PromptVia, HeadlessPromptViaStdin)
	}

	// codex is deliberately absent: `codex exec` rejects --ask-for-approval, which
	// the permission table gives every codex child at both unattended tiers, so a
	// row here would only build command lines that fail to parse. shell runs no AI
	// at all. Both stay out until something changes on purpose (子 plan 内部 C5).
	for _, provider := range []string{"codex", "shell", "", "not-a-provider"} {
		if _, ok := HeadlessDefFor(provider, nil); ok {
			t.Errorf("HeadlessDefFor(%q) = true, want no definition", provider)
		}
	}
}

// The rest of the built-in providers run their print mode through the generic
// "text" parser: the exit code is the verdict and the CLI's own output is what
// the session shows. The test pins the shape every such row must have — it
// validates, it names the text format, and because none of these CLIs documents
// reading the prompt from stdin, the prompt is an argument.
func TestHeadlessDefForTextModeBuiltins(t *testing.T) {
	for _, provider := range []string{"grok", "cursor-agent", "opencode", "copilot", "command-code"} {
		def, ok := HeadlessDefFor(provider, nil)
		if !ok {
			t.Errorf("HeadlessDefFor(%q) = false, want the built-in definition", provider)
			continue
		}
		if err := ValidateHeadlessDef(def); err != nil {
			t.Errorf("the built-in %s definition does not validate: %v", provider, err)
		}
		if def.Format != HeadlessFormatText {
			t.Errorf("%s format = %q, want %q", provider, def.Format, HeadlessFormatText)
		}
		if def.PromptVia != HeadlessPromptViaArg {
			t.Errorf("%s prompt_via = %q, want %q", provider, def.PromptVia, HeadlessPromptViaArg)
		}
		if len(def.Args) == 0 {
			t.Errorf("%s has no flags selecting print mode", provider)
		}
	}
}

// A row must not carry permission flags. Those are appended by the same code the
// interactive launch uses, and a second copy here would be the one that drifts
// (親 plan からの差分 5 / 不変条件 5).
func TestBuiltinHeadlessDefsCarryNoPermissionFlags(t *testing.T) {
	forbidden := []string{
		"--permission-mode", "--allowedTools", "--allow-tool", "--allow-all",
		"--ask-for-approval", "--sandbox", "--yolo", "--force", "--auto",
		"--always-approve", "--dangerously-bypass-approvals-and-sandbox",
		"--model", "--effort", "--reasoning-effort", "--variant",
	}
	for provider, def := range builtinHeadlessDefs {
		for _, arg := range def.Args {
			for _, bad := range forbidden {
				if arg == bad {
					t.Errorf("%s headless args carry %q; model / effort / permission flags come from the shared launch path", provider, bad)
				}
			}
		}
	}
}

// The table is never handed out by reference.
func TestHeadlessDefForReturnsCopy(t *testing.T) {
	def, _ := HeadlessDefFor("claude", nil)
	def.Args[0] = "tampered"
	again, _ := HeadlessDefFor("claude", nil)
	if again.Args[0] == "tampered" {
		t.Error("HeadlessDefFor returned the table's own slice")
	}
}

// A custom provider carries the same definition shape, which is what lets a CLI
// many-ai-cli has never heard of become an unattended child (親 plan D9).
func TestHeadlessDefForCustomProvider(t *testing.T) {
	cfg := &Config{CustomProviders: CustomProviders{
		{ID: "my-cli", Command: "my-cli", Headless: &HeadlessDef{
			Args: []string{"--print"}, Format: HeadlessFormatText,
		}},
		{ID: "plain-cli", Command: "plain-cli"},
		{ID: "broken-cli", Command: "broken-cli", Headless: &HeadlessDef{
			Args: []string{"--print"}, Format: "made-up-format",
		}},
	}}
	def, ok := HeadlessDefFor("my-cli", cfg)
	if !ok {
		t.Fatal("HeadlessDefFor(\"my-cli\") = false, want the config definition")
	}
	if def.Format != HeadlessFormatText || def.PromptVia != HeadlessPromptViaStdin {
		t.Errorf("my-cli def = %+v, want text format with the stdin default", def)
	}
	// No headless block at all: interactive only, and no warning about it.
	if _, ok := HeadlessDefFor("plain-cli", cfg); ok {
		t.Error("HeadlessDefFor(\"plain-cli\") = true, want no definition")
	}
	// A malformed definition is ignored rather than passed on to a command line.
	if _, ok := HeadlessDefFor("broken-cli", cfg); ok {
		t.Error("HeadlessDefFor(\"broken-cli\") = true, want the invalid definition ignored")
	}
	warnings := strings.Join(cfg.customProviderWarnings(), "\n")
	if !strings.Contains(warnings, "broken-cli") || !strings.Contains(warnings, "headless") {
		t.Errorf("warnings = %q, want the broken headless definition named", warnings)
	}
	if strings.Contains(warnings, "plain-cli") {
		t.Errorf("warnings = %q, want nothing about an entry with no headless block", warnings)
	}
}

func TestValidateHeadlessDef(t *testing.T) {
	ok := []HeadlessDef{
		{Format: HeadlessFormatText},
		{Args: []string{"run", "--format", "json"}, Format: HeadlessFormatText, PromptVia: HeadlessPromptViaArg},
		{Args: []string{"-p"}, Format: HeadlessFormatClaudeStreamJSON, PromptVia: HeadlessPromptViaStdin},
	}
	for _, def := range ok {
		if err := ValidateHeadlessDef(def); err != nil {
			t.Errorf("ValidateHeadlessDef(%+v) = %v, want nil", def, err)
		}
	}
	bad := []HeadlessDef{
		{},               // no format
		{Format: "yaml"}, // unknown format
		{Format: HeadlessFormatText, PromptVia: "file"},        // unknown prompt route
		{Format: HeadlessFormatText, Args: []string{""}},       // empty entry
		{Format: HeadlessFormatText, Args: []string{"-p\n-x"}}, // control character
		{Format: HeadlessFormatText, Args: []string{strings.Repeat("x", MaxHeadlessArgLen+1)}},
		{Format: HeadlessFormatText, Args: make([]string, MaxHeadlessArgs+1)},
	}
	for _, def := range bad {
		if err := ValidateHeadlessDef(def); err == nil {
			t.Errorf("ValidateHeadlessDef(%+v) = nil, want an error", def)
		}
	}
}

// The resolution is a pure function so the table below is the whole rule
// (元設計 8 節 / 親 plan D2).
func TestResolveExecutionMode(t *testing.T) {
	cases := []struct {
		name       string
		requested  string
		capable    bool
		origin     string
		unattended bool
		want       string
		wantErr    bool
	}{
		// 不変条件 1: an omitted mode stays omitted, whoever asks.
		{name: "unset conductor", requested: "", capable: true, origin: LaunchOriginConductor, want: ExecutionModeUnset},
		{name: "unset ui", requested: "", capable: true, origin: LaunchOriginUI, want: ExecutionModeUnset},
		{name: "unset relay", requested: "", capable: true, unattended: true, want: ExecutionModeUnset},

		{name: "explicit interactive stays", requested: ExecutionModeInteractive, capable: true, want: ExecutionModeInteractive},
		{name: "explicit interactive without support", requested: ExecutionModeInteractive, capable: false, want: ExecutionModeInteractive},

		// D2: explicit headless is honoured or refused, never downgraded.
		{name: "explicit headless capable", requested: ExecutionModeHeadless, capable: true, origin: LaunchOriginUI, want: ExecutionModeHeadless},
		{name: "explicit headless incapable", requested: ExecutionModeHeadless, capable: false, wantErr: true},

		{name: "auto unattended capable", requested: ExecutionModeAuto, capable: true, origin: LaunchOriginConductor, want: ExecutionModeHeadless},
		{name: "auto relay capable", requested: ExecutionModeAuto, capable: true, origin: LaunchOriginUI, unattended: true, want: ExecutionModeHeadless},
		{name: "auto unattended incapable", requested: ExecutionModeAuto, capable: false, origin: LaunchOriginConductor, want: ExecutionModeInteractive},
		{name: "auto from the screen", requested: ExecutionModeAuto, capable: true, origin: LaunchOriginUI, want: ExecutionModeInteractive},

		{name: "unknown value", requested: "batch", capable: true, wantErr: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := ResolveExecutionMode(tc.requested, tc.capable, tc.origin, tc.unattended)
			if tc.wantErr {
				if err == nil {
					t.Fatalf("ResolveExecutionMode(%q, %v, %q, %v) = %q, want an error", tc.requested, tc.capable, tc.origin, tc.unattended, got)
				}
				return
			}
			if err != nil {
				t.Fatalf("ResolveExecutionMode(%q, %v, %q, %v) = %v", tc.requested, tc.capable, tc.origin, tc.unattended, err)
			}
			if got != tc.want {
				t.Errorf("ResolveExecutionMode(%q, %v, %q, %v) = %q, want %q", tc.requested, tc.capable, tc.origin, tc.unattended, got, tc.want)
			}
		})
	}
	if IsHeadlessExecutionMode(ExecutionModeAuto) {
		t.Error("an unresolved auto must not read as headless")
	}
	if !IsHeadlessExecutionMode(ExecutionModeHeadless) {
		t.Error("headless must read as headless")
	}
}

func TestChildExecutionModeDefault(t *testing.T) {
	// Unset is today's behaviour and must stay that way.
	if got := (OrchestrationConfig{}).ChildExecutionModeDefault(); got != ExecutionModeUnset {
		t.Errorf("unset default = %q, want empty", got)
	}
	for _, mode := range []string{ExecutionModeAuto, ExecutionModeInteractive, ExecutionModeHeadless} {
		o := OrchestrationConfig{ChildExecutionMode: mode}
		if got := o.ChildExecutionModeDefault(); got != mode {
			t.Errorf("ChildExecutionModeDefault(%q) = %q, want %q", mode, got, mode)
		}
	}
	// A value this build does not know falls back to today's behaviour and says so.
	cfg := &Config{}
	cfg.Orchestration.ChildExecutionMode = "batch"
	if got := cfg.Orchestration.ChildExecutionModeDefault(); got != ExecutionModeUnset {
		t.Errorf("unknown default = %q, want empty", got)
	}
	warnings := strings.Join(cfg.childExecutionModeWarnings(), "\n")
	if !strings.Contains(warnings, "child_execution_mode") {
		t.Errorf("warnings = %q, want the setting named", warnings)
	}
	clean := &Config{}
	clean.Orchestration.ChildExecutionMode = ExecutionModeAuto
	if w := clean.childExecutionModeWarnings(); len(w) != 0 {
		t.Errorf("warnings for a valid value = %v, want none", w)
	}
}

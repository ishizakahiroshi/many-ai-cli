package config

import (
	"strings"
	"testing"
)

func TestSplitCommandLineBasic(t *testing.T) {
	for _, tc := range []struct {
		name string
		in   string
		want []string
	}{
		{"simple args", "my-cli --agent x", []string{"my-cli", "--agent", "x"}},
		{"collapses runs of spaces and tabs", "my-cli  --agent\t\tx", []string{"my-cli", "--agent", "x"}},
		{"trims leading and trailing delimiters", "  my-cli --agent  ", []string{"my-cli", "--agent"}},
		{"backslash is literal, not an escape", `C:\a\b.exe`, []string{`C:\a\b.exe`}},
		{
			"quoted span with embedded spaces, mid-token quoting",
			`"C:\Program Files\my cli\cli.exe" --flag`,
			[]string{`C:\Program Files\my cli\cli.exe`, "--flag"},
		},
		{"doubled quote is a literal quote", `--flag="a""b"`, []string{`--flag=a"b`}},
		{"lone empty quoted argument", `mycli ""`, []string{"mycli", ""}},
		{"single quote has no special meaning", `it's-fine`, []string{`it's-fine`}},
		{"env vars, tilde, glob, shell operators pass through unexpanded",
			`mycli $HOME %APPDATA% ~ * | && ;`,
			[]string{"mycli", "$HOME", "%APPDATA%", "~", "*", "|", "&&", ";"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := SplitCommandLine(tc.in)
			if err != nil {
				t.Fatalf("SplitCommandLine(%q) error = %v", tc.in, err)
			}
			if len(got) != len(tc.want) {
				t.Fatalf("SplitCommandLine(%q) = %#v, want %#v", tc.in, got, tc.want)
			}
			for i := range got {
				if got[i] != tc.want[i] {
					t.Fatalf("SplitCommandLine(%q)[%d] = %q, want %q", tc.in, i, got[i], tc.want[i])
				}
			}
		})
	}
}

func TestSplitCommandLineErrors(t *testing.T) {
	for _, tc := range []struct {
		name string
		in   string
		want string
	}{
		{"unterminated quote", `my-cli "unterminated`, "unterminated"},
		{"control character", "my-cli\nnewline", "control character"},
		{"empty string", "", "empty"},
		{"whitespace only", "   \t  ", "empty"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := SplitCommandLine(tc.in)
			if err == nil {
				t.Fatalf("SplitCommandLine(%q) = nil error, want one mentioning %q", tc.in, tc.want)
			}
			if !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("SplitCommandLine(%q) error = %q, want it to mention %q", tc.in, err.Error(), tc.want)
			}
		})
	}
}

func TestCustomProviderArgvUsesSplitCommandLine(t *testing.T) {
	got, err := (CustomProvider{Command: "my-cli --agent"}).Argv()
	if err != nil {
		t.Fatalf("Argv() error = %v", err)
	}
	if len(got) != 2 || got[0] != "my-cli" || got[1] != "--agent" {
		t.Fatalf("Argv() = %#v, want [my-cli --agent]", got)
	}
}

func TestValidateCustomProviderIDRejectsShell(t *testing.T) {
	if err := ValidateCustomProviderID("shell"); err == nil {
		t.Fatal("ValidateCustomProviderID(\"shell\") = nil, want an error (reserved id)")
	}
	if IsBuiltinProviderID("shell") {
		t.Fatal(`IsBuiltinProviderID("shell") = true, want false — "shell" is reserved but not a doctor probe target`)
	}
	if !IsReservedProviderID("shell") {
		t.Fatal(`IsReservedProviderID("shell") = false, want true`)
	}
	if !IsReservedProviderID("SHELL") {
		t.Fatal(`IsReservedProviderID("SHELL") = false, want true (case-insensitive)`)
	}
}

// TestBuiltinProviderIDsUnchanged pins the exact set doctor.providers() probes
// with LookPath+--version. Adding "shell" (or anything else) to it would make
// doctor wrongly LookPath-probe a synthetic identity — see BuiltinProviderIDs'
// doc comment.
func TestBuiltinProviderIDsUnchanged(t *testing.T) {
	want := []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code"}
	if len(BuiltinProviderIDs) != len(want) {
		t.Fatalf("BuiltinProviderIDs = %#v, want %#v", BuiltinProviderIDs, want)
	}
	for i := range want {
		if BuiltinProviderIDs[i] != want[i] {
			t.Fatalf("BuiltinProviderIDs = %#v, want %#v", BuiltinProviderIDs, want)
		}
	}
}

func TestConfigIsCustomProviderID(t *testing.T) {
	cfg := &Config{CustomProviders: CustomProviders{
		{ID: "my-cli", Command: "my-cli"},
		{ID: "claude", Command: "claude"}, // built-in と衝突するので EffectiveCustomProviders から落ちる
	}}
	if !cfg.IsCustomProviderID("my-cli") {
		t.Fatal("IsCustomProviderID(\"my-cli\") = false, want true")
	}
	if !cfg.IsCustomProviderID("MY-CLI") {
		t.Fatal("IsCustomProviderID(\"MY-CLI\") = false, want true (case-insensitive)")
	}
	if cfg.IsCustomProviderID("claude") {
		t.Fatal("IsCustomProviderID(\"claude\") = true, want false (dropped by EffectiveCustomProviders)")
	}
	if cfg.IsCustomProviderID("unknown-cli") {
		t.Fatal("IsCustomProviderID(\"unknown-cli\") = true, want false")
	}
	if (*Config)(nil).IsCustomProviderID("my-cli") {
		t.Fatal("IsCustomProviderID on a nil *Config must return false, not panic")
	}
}

package provider

import "testing"

func TestResolveUpdateArgvWithoutUpdateIsUnconfigured(t *testing.T) {
	definition := Definition{
		Launch: &LaunchDefinition{Executable: "claude"},
	}
	argv, reason := ResolveUpdateArgv(definition, "claude")
	if argv != nil {
		t.Fatalf("expected no argv, got %#v", argv)
	}
	if reason == "" {
		t.Fatal("expected a non-empty unavailable reason")
	}
}

func TestResolveUpdateArgvHonorsSelectedExecutableCandidate(t *testing.T) {
	// Mirrors the copilot manifest: two launch candidates, only the primary
	// one has an update method.
	definition := Definition{
		Launch: &LaunchDefinition{ExecutableCandidates: []string{"copilot", "gh"}},
		Update: &UpdateDefinition{Args: []string{"update"}},
	}

	argv, reason := ResolveUpdateArgv(definition, "copilot")
	if reason != "" {
		t.Fatalf("expected no unavailable reason for primary candidate, got %q", reason)
	}
	want := []string{"copilot", "update"}
	if len(argv) != len(want) || argv[0] != want[0] || argv[1] != want[1] {
		t.Fatalf("argv = %#v, want %#v", argv, want)
	}

	argv, reason = ResolveUpdateArgv(definition, "gh")
	if argv != nil {
		t.Fatalf("expected no argv for fallback candidate, got %#v", argv)
	}
	if reason == "" {
		t.Fatal("expected an unavailable reason for the fallback candidate")
	}
}

func TestResolveUpdateArgvUsesExplicitExecutableRegardlessOfCandidate(t *testing.T) {
	definition := Definition{
		Launch: &LaunchDefinition{ExecutableCandidates: []string{"copilot", "gh"}},
		Update: &UpdateDefinition{Args: []string{"update"}, Executable: "copilot"},
	}
	argv, reason := ResolveUpdateArgv(definition, "gh")
	if reason != "" {
		t.Fatalf("expected explicit update.executable to override candidate mismatch, got reason %q", reason)
	}
	want := []string{"copilot", "update"}
	if len(argv) != len(want) || argv[0] != want[0] || argv[1] != want[1] {
		t.Fatalf("argv = %#v, want %#v", argv, want)
	}
}

func TestResolveVersionArgsDefaultsToVersionFlag(t *testing.T) {
	if got := ResolveVersionArgs(nil); len(got) != 1 || got[0] != "--version" {
		t.Fatalf("ResolveVersionArgs(nil) = %#v, want [--version]", got)
	}
	custom := &UpdateDefinition{VersionArgs: []string{"version"}}
	if got := ResolveVersionArgs(custom); len(got) != 1 || got[0] != "version" {
		t.Fatalf("ResolveVersionArgs(custom) = %#v, want [version]", got)
	}
}

func TestResolveUpdateTimeoutSecondsDefaults(t *testing.T) {
	if got := ResolveUpdateTimeoutSeconds(nil); got != defaultUpdateTimeoutSec {
		t.Fatalf("ResolveUpdateTimeoutSeconds(nil) = %d, want %d", got, defaultUpdateTimeoutSec)
	}
	custom := &UpdateDefinition{TimeoutSeconds: 30}
	if got := ResolveUpdateTimeoutSeconds(custom); got != 30 {
		t.Fatalf("ResolveUpdateTimeoutSeconds(custom) = %d, want 30", got)
	}
}

func TestUpdateEnabledDefaultsToArgsPresence(t *testing.T) {
	if UpdateEnabled(nil) {
		t.Fatal("UpdateEnabled(nil) should be false")
	}
	if UpdateEnabled(&UpdateDefinition{}) {
		t.Fatal("UpdateEnabled with no args should default to false")
	}
	if !UpdateEnabled(&UpdateDefinition{Args: []string{"update"}}) {
		t.Fatal("UpdateEnabled with args should default to true")
	}
	disabled := false
	if UpdateEnabled(&UpdateDefinition{Args: []string{"update"}, Enabled: &disabled}) {
		t.Fatal("explicit enabled:false must override the args-based default")
	}
}

func TestValidateDefinitionRejectsUpdateEnabledWithoutArgs(t *testing.T) {
	raw := []byte(`{"schema_version":1,"id":"test-cli","display_name":"Test","launch":{"executable":"test"},"update":{"enabled":true}}`)
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition error: %v", err)
	}
	if !contains(diagnosticCodes(diagnostics), "update_enabled_without_args") {
		t.Fatalf("expected update_enabled_without_args, got %#v", diagnostics)
	}
}

func TestValidateDefinitionAcceptsWellFormedUpdate(t *testing.T) {
	raw := []byte(`{"schema_version":1,"id":"test-cli","display_name":"Test","launch":{"executable":"test"},"update":{"args":["update"],"enabled":true,"timeout_seconds":60}}`)
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition error: %v", err)
	}
	for _, d := range diagnostics {
		if d.IsError() {
			t.Fatalf("unexpected error diagnostic: %#v", d)
		}
	}
}

func TestValidateDefinitionRejectsUpdateTimeoutOutOfRange(t *testing.T) {
	raw := []byte(`{"schema_version":1,"id":"test-cli","display_name":"Test","launch":{"executable":"test"},"update":{"timeout_seconds":9999}}`)
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition error: %v", err)
	}
	if !contains(diagnosticCodes(diagnostics), "invalid_update_timeout") {
		t.Fatalf("expected invalid_update_timeout, got %#v", diagnostics)
	}
}

func TestCloneDefinitionDeepCopiesUpdate(t *testing.T) {
	original := Definition{
		Launch: &LaunchDefinition{Executable: "claude"},
		Update: &UpdateDefinition{Args: []string{"update"}},
	}
	clone := cloneDefinition(original)
	clone.Update.Args[0] = "mutated"

	if original.Update.Args[0] != "update" {
		t.Fatalf("cloning the definition mutated the original update.args: %#v", original.Update.Args)
	}
}

func diagnosticCodes(diagnostics []Diagnostic) []string {
	out := make([]string, 0, len(diagnostics))
	for _, d := range diagnostics {
		out = append(out, d.Code)
	}
	return out
}

package config

import "testing"

// Every row must offer at least one level, or the UI has a field it cannot
// populate. Rows that accept free-form levels still carry suggestions.
func TestEffortTableRowsHaveLevelsAndArgs(t *testing.T) {
	for _, provider := range EffortProviders() {
		row, ok := EffortSupportFor(provider)
		if !ok {
			t.Fatalf("EffortSupportFor(%q) = false for a provider listed by EffortProviders", provider)
		}
		if len(row.Levels) == 0 {
			t.Errorf("provider %q has no effort levels", provider)
		}
		if row.Args == nil {
			t.Fatalf("provider %q has no Args function", provider)
		}
		for _, level := range row.Levels {
			if err := ValidateEffort(provider, level); err != nil {
				t.Errorf("ValidateEffort(%q, %q) = %v, want nil", provider, level, err)
			}
			if args := EffortArgs(provider, level); len(args) == 0 {
				t.Errorf("EffortArgs(%q, %q) returned no arguments", provider, level)
			}
		}
	}
}

// Providers without a row must stay without one: an unknown id, a built-in
// CLI that has no effort flag, and a user-defined custom provider all report
// "no mapping" so no launch path can invent arguments for them.
func TestEffortSupportForReportsNoMapping(t *testing.T) {
	noMapping := []string{
		"copilot", "grok", "cursor-agent", "command-code", "shell",
		"my-local-cli", "", "CLAUDE",
	}
	for _, provider := range noMapping {
		if _, ok := EffortSupportFor(provider); ok {
			t.Errorf("EffortSupportFor(%q) = true, want false", provider)
		}
		if err := ValidateEffort(provider, "high"); err == nil {
			t.Errorf("ValidateEffort(%q, \"high\") = nil, want an error", provider)
		}
		if args := EffortArgs(provider, "high"); args != nil {
			t.Errorf("EffortArgs(%q, \"high\") = %v, want nil", provider, args)
		}
	}
}

// Omitting effort must stay free for every provider, mapped or not: that is
// what keeps existing callers byte-for-byte unchanged.
func TestValidateEffortAcceptsEmptyForEveryProvider(t *testing.T) {
	for _, provider := range []string{"claude", "codex", "opencode", "copilot", "shell", "my-local-cli"} {
		if err := ValidateEffort(provider, ""); err != nil {
			t.Errorf("ValidateEffort(%q, \"\") = %v, want nil", provider, err)
		}
		if args := EffortArgs(provider, ""); args != nil {
			t.Errorf("EffortArgs(%q, \"\") = %v, want nil", provider, args)
		}
	}
}

func TestValidateEffortRejectsUnknownLevelOnFixedLadder(t *testing.T) {
	for _, level := range []string{"turbo", "HIGH", "minimal"} {
		if err := ValidateEffort("claude", level); err == nil {
			t.Errorf("ValidateEffort(\"claude\", %q) = nil, want an error", level)
		}
	}
	if err := ValidateEffort("codex", "minimal"); err != nil {
		t.Errorf("ValidateEffort(\"codex\", \"minimal\") = %v, want nil", err)
	}
}

// opencode takes variant names from the model definition, so an unlisted
// value is accepted — but only if it still looks like a level.
func TestValidateEffortFreeFormProviderChecksShapeOnly(t *testing.T) {
	if err := ValidateEffort("opencode", "thinking-2"); err != nil {
		t.Errorf("ValidateEffort(\"opencode\", \"thinking-2\") = %v, want nil", err)
	}
	rejected := []string{
		"--permission-mode",
		"high high",
		"high;rm",
		"high$(id)",
		"0123456789012345678901234567890123",
	}
	for _, level := range rejected {
		if err := ValidateEffort("opencode", level); err == nil {
			t.Errorf("ValidateEffort(\"opencode\", %q) = nil, want an error", level)
		}
	}
}

func TestEffortArgsShape(t *testing.T) {
	cases := []struct {
		provider string
		level    string
		want     []string
	}{
		{"claude", "high", []string{"--effort", "high"}},
		{"codex", "xhigh", []string{"-c", `model_reasoning_effort="xhigh"`}},
		{"opencode", "low", []string{"--variant", "low"}},
	}
	for _, tc := range cases {
		got := EffortArgs(tc.provider, tc.level)
		if len(got) != len(tc.want) {
			t.Fatalf("EffortArgs(%q, %q) = %v, want %v", tc.provider, tc.level, got, tc.want)
		}
		for i := range got {
			if got[i] != tc.want[i] {
				t.Fatalf("EffortArgs(%q, %q) = %v, want %v", tc.provider, tc.level, got, tc.want)
			}
		}
	}
}

// EffortLevelsFor hands out a copy: a caller that sorts or truncates the
// slice must not rewrite the table.
func TestEffortLevelsForReturnsCopy(t *testing.T) {
	levels := EffortLevelsFor("claude")
	if len(levels) == 0 {
		t.Fatal("EffortLevelsFor(\"claude\") returned no levels")
	}
	levels[0] = "tampered"
	if again := EffortLevelsFor("claude"); again[0] == "tampered" {
		t.Error("EffortLevelsFor returned the table's own slice")
	}
	if EffortLevelsFor("copilot") != nil {
		t.Error("EffortLevelsFor(\"copilot\") should be nil (no mapping)")
	}
}

func TestValidateExecutionMode(t *testing.T) {
	// headless joined the schema check when the headless definition table
	// landed (子 plan plan_child_execution_modes_headless.md 内部 C1). Whether a
	// given provider can honour it is decided per launch by
	// ResolveExecutionMode, not here.
	for _, mode := range []string{"", "auto", "interactive", "headless"} {
		if err := ValidateExecutionMode(mode); err != nil {
			t.Errorf("ValidateExecutionMode(%q) = %v, want nil", mode, err)
		}
	}
	for _, mode := range []string{"batch", "AUTO", " auto"} {
		if err := ValidateExecutionMode(mode); err == nil {
			t.Errorf("ValidateExecutionMode(%q) = nil, want an error", mode)
		}
	}
	if got := NormalizeExecutionMode("  auto "); got != "auto" {
		t.Errorf("NormalizeExecutionMode(\"  auto \") = %q, want \"auto\"", got)
	}
}

func TestValidatePermissionPreset(t *testing.T) {
	// bounded joined attended/full when the tier table landed (子 plan
	// plan_derived-session-launch_c2_permission-tiers.md 内部 C3). 段 2 を持たない
	// provider でも受理する: 要求は通り、段 3 へ落ちたことを確認ダイアログが見せる。
	for _, preset := range []string{"", "attended", "bounded", "full"} {
		if err := ValidatePermissionPreset(preset); err != nil {
			t.Errorf("ValidatePermissionPreset(%q) = %v, want nil", preset, err)
		}
	}
	for _, preset := range []string{"none", "FULL", " full"} {
		if err := ValidatePermissionPreset(preset); err == nil {
			t.Errorf("ValidatePermissionPreset(%q) = nil, want an error", preset)
		}
	}
	if got := NormalizePermissionPreset(" full\t"); got != "full" {
		t.Errorf("NormalizePermissionPreset(\" full\\t\") = %q, want \"full\"", got)
	}
}

// The schema lists more than this build accepts; the gap is what later plan
// phases enable. If a value becomes available, it must be removed from this
// expectation deliberately.
func TestAvailableSetsAreSubsetsOfSchema(t *testing.T) {
	assertSubset := func(name string, available, known []string) {
		for _, value := range available {
			found := false
			for _, k := range known {
				if k == value {
					found = true
					break
				}
			}
			if !found {
				t.Errorf("%s: %q is available but not in the schema", name, value)
			}
		}
	}
	assertSubset("execution mode", AvailableExecutionModes(), KnownExecutionModes())
	assertSubset("permission preset", AvailablePermissionPresets(), KnownPermissionPresets())
	// 実行モードも 3 つとも選べる（headless は子 plan
	// plan_child_execution_modes_headless.md 内部 C1 で解禁した）。provider ごとの
	// 可否は ResolveExecutionMode が起動ごとに決める。
	if len(AvailableExecutionModes()) != len(KnownExecutionModes()) {
		t.Errorf("execution mode: available = %v, want every schema value", AvailableExecutionModes())
	}
	// 権限の段は 3 つとも選べる（内部 C3 で bounded を解禁した）。実行モードと違い、
	// ここに「まだ選べない値」は残っていない。
	if len(AvailablePermissionPresets()) != len(KnownPermissionPresets()) {
		t.Errorf("permission preset: available = %v, want every schema value", AvailablePermissionPresets())
	}
}

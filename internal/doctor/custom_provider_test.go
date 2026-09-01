package doctor

import (
	"context"
	"errors"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestCustomProvidersAbsentIsSilent(t *testing.T) {
	cfg := &config.Config{}
	if got := customProviders(cfg); got != nil {
		t.Fatalf("customProviders() = %#v, want nil when custom_providers is unset", got)
	}
}

func TestCustomProvidersReportsUsageAndResponsibility(t *testing.T) {
	old := providerLookPath
	t.Cleanup(func() { providerLookPath = old })
	providerLookPath = func(name string) (string, error) { return "/usr/bin/" + name, nil }

	cfg := &config.Config{CustomProviders: config.CustomProviders{
		{ID: "my-cli", Command: "my-cli"},
		{ID: "other-cli", Command: "other-cli"},
	}}
	checks := customProviders(cfg)
	if len(checks) != 2 {
		t.Fatalf("checks = %#v, want 2 rows (usage + path)", checks)
	}
	got := checks[0]
	if got.Level != OK {
		t.Fatalf("Level = %v, want OK (informational, not an error)", got.Level)
	}
	if !strings.Contains(got.Message, "my-cli") || !strings.Contains(got.Message, "other-cli") {
		t.Fatalf("Message = %q, want it to name the configured provider ids", got.Message)
	}
	if !strings.Contains(got.Message, "利用者側") {
		t.Fatalf("Message = %q, want the built-in-vs-user responsibility note", got.Message)
	}
}

// TestCustomProvidersIgnoresEntriesDroppedByEffectiveFilter は、built-in
// 衝突などで EffectiveCustomProviders から落ちたエントリだけの config.yaml
// では何も出ないことを確認する（doctor が「使用中」と誤報しないため）。
func TestCustomProvidersIgnoresEntriesDroppedByEffectiveFilter(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{
		{ID: "claude", Command: "claude"}, // built-in と衝突するので EffectiveCustomProviders から落ちる
	}}
	if got := customProviders(cfg); got != nil {
		t.Fatalf("customProviders() = %#v, want nil when every entry is filtered out", got)
	}
}

func TestCustomProviderPathAllFoundIsOK(t *testing.T) {
	old := providerLookPath
	t.Cleanup(func() { providerLookPath = old })
	providerLookPath = func(name string) (string, error) { return "/usr/bin/" + name, nil }

	effective := config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
		{ID: "other-cli", Command: "other-cli"},
	}
	got := customProviderPath(effective)
	if got.Level != OK {
		t.Fatalf("Level = %v, want OK when every argv[0] resolves", got.Level)
	}
}

func TestCustomProviderPathMissingWarns(t *testing.T) {
	old := providerLookPath
	t.Cleanup(func() { providerLookPath = old })
	providerLookPath = func(name string) (string, error) {
		if name == "found-cli" {
			return "/usr/bin/found-cli", nil
		}
		return "", errors.New("not found")
	}

	effective := config.CustomProviders{
		{ID: "ok-entry", Command: "found-cli"},
		{ID: "missing-entry", Command: "missing-cli --flag"},
	}
	got := customProviderPath(effective)
	if got.Level != Warn {
		t.Fatalf("Level = %v, want Warn when an argv[0] is missing", got.Level)
	}
	if !strings.Contains(got.Message, "missing-entry") || strings.Contains(got.Message, "ok-entry") {
		t.Fatalf("Message = %q, want it to name only the missing entry", got.Message)
	}
}

func TestCustomProviderPathUnsplittableCommandWarns(t *testing.T) {
	effective := config.CustomProviders{
		{ID: "bad-quote", Command: `my-cli "unterminated`},
	}
	got := customProviderPath(effective)
	if got.Level != Warn || !strings.Contains(got.Message, "bad-quote") {
		t.Fatalf("customProviderPath() = %+v, want a Warn naming bad-quote", got)
	}
}

// TestCustomProviderPathNeverProbesVersion は、custom エントリに対して
// providerVersionOutput（builtin providers() の --version 実行）が一度も
// 呼ばれないことを固定する。doctor は「local, non-mutating diagnostics」を
// 掲げており、任意の利用者コマンドを実行しない。
func TestCustomProviderPathNeverProbesVersion(t *testing.T) {
	oldLookPath := providerLookPath
	oldVersionOutput := providerVersionOutput
	t.Cleanup(func() {
		providerLookPath = oldLookPath
		providerVersionOutput = oldVersionOutput
	})
	providerLookPath = func(name string) (string, error) { return "/usr/bin/" + name, nil }
	called := false
	providerVersionOutput = func(ctx context.Context, path string) ([]byte, error) {
		called = true
		return nil, nil
	}

	customProviderPath(config.CustomProviders{{ID: "my-cli", Command: "my-cli"}})
	if called {
		t.Fatal("providerVersionOutput was called for a custom provider; doctor must not execute user commands")
	}
}

func TestCustomProviderApprovalPatternNoticeOnlyWhenSet(t *testing.T) {
	if _, ok := customProviderApprovalPatternNotice(config.CustomProviders{
		{ID: "my-cli", Command: "my-cli"},
	}); ok {
		t.Fatal("customProviderApprovalPatternNotice returned a check when no entry sets approval_pattern_source")
	}

	check, ok := customProviderApprovalPatternNotice(config.CustomProviders{
		{ID: "my-cli", Command: "my-cli", ApprovalPatternSource: "~/patterns.md"},
	})
	if !ok {
		t.Fatal("customProviderApprovalPatternNotice returned no check when an entry sets approval_pattern_source")
	}
	if check.Level != Warn || !strings.Contains(check.Message, "my-cli") || !strings.Contains(check.Message, "未使用") {
		t.Fatalf("check = %+v, want a Warn naming my-cli and stating it is unused", check)
	}
}

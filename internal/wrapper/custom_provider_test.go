package wrapper

import (
	"testing"

	"many-ai-cli/internal/config"
)

func TestCustomProviderForFindsEffectiveEntry(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{
		{ID: "my-cli", Label: "My CLI", Command: "my-cli --agent"},
		{ID: "claude", Command: "claude"}, // built-in と衝突するので EffectiveCustomProviders から落ちる
	}}
	p, ok := customProviderFor(cfg, "my-cli")
	if !ok {
		t.Fatal("customProviderFor(my-cli) = false, want true")
	}
	if p.Label != "My CLI" || p.Command != "my-cli --agent" {
		t.Fatalf("customProviderFor(my-cli) = %#v, unexpected", p)
	}
	if _, ok := customProviderFor(cfg, "claude"); ok {
		t.Fatal("customProviderFor(claude) = true, want false (dropped by EffectiveCustomProviders)")
	}
	if _, ok := customProviderFor(cfg, "unknown"); ok {
		t.Fatal("customProviderFor(unknown) = true, want false")
	}
}

// TestShouldAppendModelFlag は、custom provider に --model が付かないことを
// Run() を直接呼ばずに固定する（敵対レビュー 2026-09-01 Finding A: 以前は
// `many-ai-cli wrap <custom-id> --model X` を直接叩く経路にこの抑止が無く、
// Hub 経由の spawn だけが internal/hub/spawn_handler.go の resolveSpawnModel
// で守られていた）。
func TestShouldAppendModelFlag(t *testing.T) {
	if !shouldAppendModelFlag("gpt-4", false) {
		t.Error("built-in provider with a model value should get --model")
	}
	if shouldAppendModelFlag("", false) {
		t.Error("empty model should never add --model, built-in or not")
	}
	if shouldAppendModelFlag("gpt-4", true) {
		t.Error("custom provider must never get --model, even when one was passed on the command line")
	}
	if shouldAppendModelFlag("", true) {
		t.Error("custom provider with no model should still be false")
	}
}

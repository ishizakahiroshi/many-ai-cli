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

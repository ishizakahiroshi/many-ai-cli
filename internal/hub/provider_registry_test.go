package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

func TestBuildProviderRegistryIncludesEmbeddedAndLegacyProviders(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent"}}}
	registry, diagnostics, err := buildProviderRegistry(cfg)
	if err != nil {
		t.Fatalf("buildProviderRegistry: %v", err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("buildProviderRegistry diagnostics = %#v", diagnostics)
	}
	if _, ok := registry.Lookup("claude"); !ok {
		t.Fatal("embedded claude is not in the registry")
	}
	custom, ok := registry.Lookup("my-cli")
	if !ok || custom.Launch == nil || custom.Launch.Executable != "my-cli" {
		t.Fatalf("legacy custom definition = %#v, %v", custom, ok)
	}
	if custom.EffectiveSource.Origin != "legacy" {
		t.Fatalf("custom origin = %q, want legacy", custom.EffectiveSource.Origin)
	}
}

func TestValidSpawnProviderUsesRegistrySnapshot(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{{ID: "my-cli", Command: "my-cli"}}}
	s := &Server{cfg: cfg}
	if !s.validSpawnProvider("claude") || !s.validSpawnProvider("my-cli") || s.validSpawnProvider("not-registered") {
		t.Fatal("validSpawnProvider did not use the registry's effective entries")
	}
}

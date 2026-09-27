package provider

import "testing"

func TestEmbeddedDefinitionsHaveStableBuiltinOrder(t *testing.T) {
	definitions, diagnostics, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatalf("EmbeddedDefinitions: %v", err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("embedded diagnostics = %#v", diagnostics)
	}
	if len(definitions) != len(BuiltinProviderIDs) {
		t.Fatalf("embedded definitions = %d, want %d", len(definitions), len(BuiltinProviderIDs))
	}
	for i, definition := range definitions {
		if definition.ID != BuiltinProviderIDs[i] {
			t.Fatalf("embedded definition %d = %q, want %q", i, definition.ID, BuiltinProviderIDs[i])
		}
	}
}

func TestEmbeddedDefinitionsBuildRegistry(t *testing.T) {
	definitions, _, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	registry, diagnostics := Build(Layers{Embedded: definitions}, DefaultAdapterCatalog())
	if len(diagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", diagnostics)
	}
	if len(registry.List()) != 7 {
		t.Fatalf("registry summaries = %d, want 7", len(registry.List()))
	}
}

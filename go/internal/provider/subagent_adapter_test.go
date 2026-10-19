package provider

import (
	"strings"
	"testing"
)

func TestAllCatalogSubagentAdaptersAreRegistered(t *testing.T) {
	catalog := DefaultAdapterCatalog()
	for key := range catalog.Keys {
		if !strings.HasPrefix(key, "subagent:") {
			continue
		}
		adapter, ok := LookupSubagentAdapter(key)
		if !ok {
			t.Fatalf("catalog key %q has no registered SubagentAdapter", key)
		}
		desc := adapter.Descriptor()
		if desc.Key != key {
			t.Fatalf("expected descriptor key %q, got %q", key, desc.Key)
		}
		if desc.Kind != AdapterSubagents {
			t.Fatalf("expected descriptor kind %q, got %q", AdapterSubagents, desc.Kind)
		}
	}
}

func TestValidateDefinitionRejectsUnknownSubagentsAdapter(t *testing.T) {
	raw := []byte(`{"schema_version":1,"id":"test-cli","display_name":"Test","launch":{"executable":"test"},"adapters":{"subagents":"subagent:missing-v1"}}`)
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		t.Fatalf("ValidateDefinition error: %v", err)
	}
	found := false
	for _, d := range diagnostics {
		if d.Code == "unknown_adapter" && d.Field == "adapters.subagents" {
			found = true
		}
	}
	if !found {
		t.Fatalf("expected unknown_adapter for adapters.subagents, got %#v", diagnostics)
	}
}

func TestClaudeEmbeddedDefinitionHasSubagentsCapability(t *testing.T) {
	definitions, diagnostics, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("embedded diagnostics = %#v", diagnostics)
	}
	registry, buildDiagnostics := Build(Layers{Embedded: definitions}, DefaultAdapterCatalog())
	if len(buildDiagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", buildDiagnostics)
	}
	claude, ok := registry.Lookup("claude")
	if !ok {
		t.Fatal("claude definition not found")
	}
	if claude.Adapters.Subagents != "subagent:claude-v1" {
		t.Fatalf("claude adapters.subagents = %q, want %q", claude.Adapters.Subagents, "subagent:claude-v1")
	}
	if !claude.Capabilities.Subagents {
		t.Fatal("claude Capabilities.Subagents = false, want true")
	}
}

// TestGrokEmbeddedDefinitionHasSubagentsCapabilityAndNoTranscriptAdapter is
// 親 plan docs/local/plan_subagent-tree-popup.md C4 の完了条件「grok.json の
// adapters に transcript が無いまま、サブエージェントの木だけが出る（チャット
// 欄の読み元が変わっていない）」の定義レベルでの確認。
func TestGrokEmbeddedDefinitionHasSubagentsCapabilityAndNoTranscriptAdapter(t *testing.T) {
	definitions, diagnostics, err := EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("embedded diagnostics = %#v", diagnostics)
	}
	registry, buildDiagnostics := Build(Layers{Embedded: definitions}, DefaultAdapterCatalog())
	if len(buildDiagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", buildDiagnostics)
	}
	grok, ok := registry.Lookup("grok")
	if !ok {
		t.Fatal("grok definition not found")
	}
	if grok.Adapters.Subagents != "subagent:grok-v1" {
		t.Fatalf("grok adapters.subagents = %q, want %q", grok.Adapters.Subagents, "subagent:grok-v1")
	}
	if !grok.Capabilities.Subagents {
		t.Fatal("grok Capabilities.Subagents = false, want true")
	}
	if grok.Adapters.Transcript != "" {
		t.Fatalf("grok adapters.transcript = %q, want empty (the chat-tab reader must not change)", grok.Adapters.Transcript)
	}
}

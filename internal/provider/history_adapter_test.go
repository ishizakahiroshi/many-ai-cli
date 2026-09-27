package provider

import (
	"strings"
	"testing"
)

func TestAllCatalogHistoryAdaptersAreRegistered(t *testing.T) {
	catalog := DefaultAdapterCatalog()
	for key := range catalog.Keys {
		if strings.HasPrefix(key, "history:") {
			adapter, ok := LookupHistoryAdapter(key)
			if !ok || adapter == nil {
				t.Fatalf("catalog key %q has no registered HistoryAdapter", key)
			}
			desc := adapter.Descriptor()
			if desc.Key != key {
				t.Fatalf("expected descriptor key %q, got %q", key, desc.Key)
			}
			if desc.Kind != AdapterHistory {
				t.Fatalf("expected descriptor kind %q, got %q", AdapterHistory, desc.Kind)
			}
		}
	}
}

func TestClaudeHistoryAdapterPathResolution(t *testing.T) {
	adapter, ok := LookupHistoryAdapter("history:claude-v1")
	if !ok {
		t.Fatal("claude history adapter not found")
	}

	ctx := SessionHistoryContext{
		Provider:       "claude",
		ClaudeDir:      "/custom/claude",
		AgentSessionID: "sess-abc-123",
	}
	path, ok := adapter.PathResolver().ResolvePath(ctx)
	if !ok || !strings.Contains(path, "sess-abc-123.jsonl") {
		t.Fatalf("expected resolved claude transcript path, got %q, %v", path, ok)
	}

	if !adapter.Parser().CanParse() {
		t.Fatal("expected claude history parser to be marked as CanParse")
	}
}

func TestCodexHistoryAdapterPathResolution(t *testing.T) {
	adapter, ok := LookupHistoryAdapter("history:codex-v1")
	if !ok {
		t.Fatal("codex history adapter not found")
	}

	ctx := SessionHistoryContext{
		Provider:       "codex",
		CodexHome:      "/custom/codex",
		AgentSessionID: "codex-sess-456",
	}
	path, ok := adapter.PathResolver().ResolvePath(ctx)
	if !ok || !strings.Contains(path, "codex-sess-456.jsonl") {
		t.Fatalf("expected resolved codex transcript path, got %q, %v", path, ok)
	}
}

func TestUnsupportedHistoryAdapter(t *testing.T) {
	adapter, ok := LookupHistoryAdapter("history:cursor-agent-v1")
	if !ok {
		t.Fatal("cursor-agent history adapter not found")
	}
	if adapter.Parser().CanParse() {
		t.Fatal("cursor-agent should be marked as CanParse=false")
	}
	_, ok = adapter.PathResolver().ResolvePath(SessionHistoryContext{})
	if ok {
		t.Fatal("unsupported path resolver should return false")
	}
}

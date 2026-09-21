package provider

import (
	"strings"
	"testing"
)

func TestAllCatalogUsageAdaptersAreRegistered(t *testing.T) {
	catalog := DefaultAdapterCatalog()
	for key := range catalog.Keys {
		if strings.HasPrefix(key, "usage:") {
			adapter, ok := LookupUsageAdapter(key)
			if !ok || adapter == nil {
				t.Fatalf("catalog key %q has no registered UsageAdapter", key)
			}
			desc := adapter.Descriptor()
			if desc.Key != key {
				t.Fatalf("expected descriptor key %q, got %q", key, desc.Key)
			}
			if desc.Kind != AdapterUsage {
				t.Fatalf("expected descriptor kind %q, got %q", AdapterUsage, desc.Kind)
			}
		}
	}
}

func TestAllCatalogSubscriptionAdaptersAreRegistered(t *testing.T) {
	catalog := DefaultAdapterCatalog()
	for key := range catalog.Keys {
		if strings.HasPrefix(key, "subscription:") {
			adapter, ok := LookupSubscriptionAdapter(key)
			if !ok || adapter == nil {
				t.Fatalf("catalog key %q has no registered SubscriptionAdapter", key)
			}
			desc := adapter.Descriptor()
			if desc.Key != key {
				t.Fatalf("expected descriptor key %q, got %q", key, desc.Key)
			}
			if desc.Kind != AdapterSubscription {
				t.Fatalf("expected descriptor kind %q, got %q", AdapterSubscription, desc.Kind)
			}
		}
	}
}

func TestUsageAdapterSourceKinds(t *testing.T) {
	claude, ok := LookupUsageAdapter("usage:claude-v1")
	if !ok || claude.SourceKind() != UsageSourcePushed {
		t.Fatalf("expected claude usage to be pushed, got %v, %v", ok, claude.SourceKind())
	}
	if !claude.CanDetectApproachingLimit() {
		t.Fatal("expected claude CanDetectApproachingLimit to be true")
	}

	codex, ok := LookupUsageAdapter("usage:codex-v1")
	if !ok || codex.SourceKind() != UsageSourceLocalFile {
		t.Fatalf("expected codex usage to be local-file, got %v, %v", ok, codex.SourceKind())
	}

	opencode, ok := LookupUsageAdapter("usage:opencode-v1")
	if !ok || opencode.SourceKind() != UsageSourceNone {
		t.Fatalf("expected opencode usage to be none, got %v, %v", ok, opencode.SourceKind())
	}
	if opencode.CanDetectApproachingLimit() {
		t.Fatal("expected opencode CanDetectApproachingLimit to be false")
	}
}

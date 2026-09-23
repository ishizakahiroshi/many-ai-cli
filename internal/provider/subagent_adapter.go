package provider

// SubagentAdapter names which registered subagent-tree reader a provider
// definition selects via adapters.subagents (docs/local/plan_subagent-tree-popup.md
// 方針 2). Unlike HistoryAdapter/UsageAdapter, it carries no path-resolution
// or parsing behavior of its own: the actual reader lives in Hub
// (internal/hub/subagent_tree.go's key → reader table), because it depends on
// Hub-internal session/transcript resolution (agent_log_handler.go 等) that
// this package must not import (internal/hub already imports
// internal/provider; the reverse would be a cycle). This registry exists so
// every catalog key beginning with "subagent:" is provably backed by a real,
// known provider (TestAllCatalogSubagentAdaptersAreRegistered) — the Hub-side
// table is required to implement one reader per key here (its own equivalent
// test lives in internal/hub/subagent_tree_test.go, 親 plan C3 の完了条件).
type SubagentAdapter struct {
	descriptor AdapterDescriptor
}

func (a SubagentAdapter) Descriptor() AdapterDescriptor {
	return a.descriptor
}

var subagentAdapterRegistry = map[string]SubagentAdapter{}

func registerSubagentAdapter(adapter SubagentAdapter) {
	subagentAdapterRegistry[adapter.Descriptor().Key] = adapter
}

func init() {
	// 1. subagent:claude-v1 (子 plan plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C2)
	registerSubagentAdapter(SubagentAdapter{
		descriptor: AdapterDescriptor{Key: "subagent:claude-v1", Kind: AdapterSubagents, Version: "v1", Provider: "claude"},
	})
	// 2. subagent:codex-v1 (親 plan docs/local/plan_subagent-tree-popup.md C3)
	registerSubagentAdapter(SubagentAdapter{
		descriptor: AdapterDescriptor{Key: "subagent:codex-v1", Kind: AdapterSubagents, Version: "v1", Provider: "codex"},
	})
	// 3. subagent:grok-v1 (親 plan docs/local/plan_subagent-tree-popup.md C4)
	registerSubagentAdapter(SubagentAdapter{
		descriptor: AdapterDescriptor{Key: "subagent:grok-v1", Kind: AdapterSubagents, Version: "v1", Provider: "grok"},
	})
}

// LookupSubagentAdapter returns the registered SubagentAdapter for key.
func LookupSubagentAdapter(key string) (SubagentAdapter, bool) {
	if key == "" || key == "none" || key == "unsupported" {
		return SubagentAdapter{}, false
	}
	adapter, ok := subagentAdapterRegistry[key]
	return adapter, ok
}

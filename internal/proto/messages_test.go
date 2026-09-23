package proto

import (
	"reflect"
	"testing"
)

// allowedSubagentNodeFields is the C1 completion criterion for
// docs/local/plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C1:
// SubagentNode must never gain a field that could hold prompt text, a tool
// result, or a child's reply (親 plan docs/local/plan_subagent-tree-popup.md
// 方針 3). Widening this list is a deliberate design decision — read 方針 3
// first — not a routine test fix, the same rule
// internal/handoff/handoff_test.go applies to Record.
var allowedSubagentNodeFields = map[string]bool{
	"ID":              true,
	"ParentID":        true,
	"Depth":           true,
	"Label":           true,
	"AgentType":       true,
	"Model":           true,
	"State":           true,
	"StartedAt":       true,
	"LastActivityAt":  true,
	"FinishedAt":      true,
	"ToolCalls":       true,
	"LastToolName":    true,
	"LastToolSummary": true,
}

func TestSubagentNodeFieldsAreTheAllowlist(t *testing.T) {
	typ := reflect.TypeOf(SubagentNode{})
	seen := map[string]bool{}
	for i := 0; i < typ.NumField(); i++ {
		name := typ.Field(i).Name
		seen[name] = true
		if !allowedSubagentNodeFields[name] {
			t.Errorf("SubagentNode gained an unexpected field %q. "+
				"Read 親 plan (docs/local/plan_subagent-tree-popup.md) 方針 3 before "+
				"widening the allowlist in messages.go and messages_test.go.", name)
		}
	}
	for name := range allowedSubagentNodeFields {
		if !seen[name] {
			t.Errorf("allowlist names field %q which no longer exists on SubagentNode; trim messages_test.go", name)
		}
	}
}

package hub

import "testing"

func TestEvaluateCodexRisk(t *testing.T) {
	cases := []struct {
		name         string
		currentModel string
		nextModel    string
		sandbox      string
		approval     string
		wantHighRisk bool
	}{
		{name: "unchanged default", currentModel: "gpt-5", nextModel: "gpt-5", wantHighRisk: false},
		{name: "empty next model", currentModel: "gpt-5", nextModel: " ", wantHighRisk: false},
		{name: "model changed", currentModel: "gpt-5", nextModel: "gpt-5-codex", wantHighRisk: true},
		{name: "danger sandbox", currentModel: "gpt-5", nextModel: "gpt-5", sandbox: "danger-full-access", wantHighRisk: true},
		{name: "approval never", currentModel: "gpt-5", nextModel: "gpt-5", approval: "never", wantHighRisk: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := evaluateCodexRisk(tc.currentModel, tc.nextModel, tc.sandbox, tc.approval)
			if got.HighRisk != tc.wantHighRisk {
				t.Fatalf("HighRisk = %v, want %v", got.HighRisk, tc.wantHighRisk)
			}
		})
	}
}

func TestEvaluateClaudeRisk(t *testing.T) {
	cases := []struct {
		name           string
		currentModel   string
		nextModel      string
		permissionMode string
		wantHighRisk   bool
	}{
		{name: "unchanged default", currentModel: "sonnet", nextModel: "sonnet", wantHighRisk: false},
		{name: "empty next model", currentModel: "sonnet", nextModel: "", wantHighRisk: false},
		{name: "model changed", currentModel: "sonnet", nextModel: "opus", wantHighRisk: true},
		{name: "bypass permissions", currentModel: "sonnet", nextModel: "sonnet", permissionMode: "bypassPermissions", wantHighRisk: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := evaluateClaudeRisk(tc.currentModel, tc.nextModel, tc.permissionMode)
			if got.HighRisk != tc.wantHighRisk {
				t.Fatalf("HighRisk = %v, want %v", got.HighRisk, tc.wantHighRisk)
			}
		})
	}
}

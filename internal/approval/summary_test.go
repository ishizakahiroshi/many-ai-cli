package approval

import (
	"reflect"
	"testing"

	"many-ai-cli/internal/proto"
)

func TestSummarizeExtractsCommandPathsAndRisk(t *testing.T) {
	got := Summarize("Command requires approval", "Command requires approval\nRun: rm -rf ./dist ./tmp\n")
	if got.Command != "rm -rf ./dist ./tmp" {
		t.Fatalf("command = %q", got.Command)
	}
	if got.Risk != proto.ApprovalRiskHigh {
		t.Fatalf("risk = %q, want high", got.Risk)
	}
	if want := []string{"./dist", "./tmp"}; !reflect.DeepEqual(got.Paths, want) {
		t.Fatalf("paths = %#v, want %#v", got.Paths, want)
	}
}

func TestClassifyRiskIsConservative(t *testing.T) {
	tests := []struct {
		command string
		want    proto.ApprovalRiskTier
	}{
		{"git status", proto.ApprovalRiskLow},
		{"git commit -m test", proto.ApprovalRiskMid},
		{"git push --force origin main", proto.ApprovalRiskHigh},
		{"", proto.ApprovalRiskMid},
	}
	for _, tt := range tests {
		if got := ClassifyRisk(tt.command); got != tt.want {
			t.Errorf("ClassifyRisk(%q) = %q, want %q", tt.command, got, tt.want)
		}
	}
}

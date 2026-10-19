package notify

import (
	"strings"
	"testing"
)

func TestNtfyApprovalActionsExcludeHighRiskApprove(t *testing.T) {
	payload := ApprovalPayload{
		Risk:       "high",
		ApproveURL: "https://tail.example/api/approval-action/approve",
		RejectURL:  "https://tail.example/api/approval-action/reject",
		OpenURL:    "https://tail.example/?session_id=1",
	}
	actions := ntfyApprovalActions(payload)
	if strings.Contains(actions, "Approve") {
		t.Fatalf("high-risk action header contains Approve: %q", actions)
	}
	if !strings.Contains(actions, "Reject") || !strings.Contains(actions, "Open") {
		t.Fatalf("actions = %q, want Reject and Open", actions)
	}
}

func TestNtfyApprovalActionsRejectUnsafeURLs(t *testing.T) {
	actions := ntfyApprovalActions(ApprovalPayload{ApproveURL: "/api/approval-action/token", RejectURL: "javascript:alert(1)"})
	if actions != "" {
		t.Fatalf("actions = %q, want no unsafe action", actions)
	}
}

package provider

import (
	"strings"
	"testing"

	"many-ai-cli/internal/proto"
)

func TestAllCatalogApprovalAdaptersAreRegistered(t *testing.T) {
	catalog := DefaultAdapterCatalog()
	for key := range catalog.Keys {
		if strings.HasPrefix(key, "approval:") {
			adapter, ok := LookupApprovalAdapter(key)
			if !ok || adapter == nil {
				t.Fatalf("catalog key %q has no registered ApprovalAdapter", key)
			}
			desc := adapter.Descriptor()
			if desc.Key != key {
				t.Fatalf("expected descriptor key %q, got %q", key, desc.Key)
			}
			if desc.Kind != AdapterApproval {
				t.Fatalf("expected descriptor kind %q, got %q", AdapterApproval, desc.Kind)
			}
		}
	}
}

func TestApprovalAdapterRememberKey(t *testing.T) {
	claudeAdapter, ok := LookupApprovalAdapter("approval:claude-v1")
	if !ok {
		t.Fatal("claude adapter not found")
	}
	key := claudeAdapter.RememberKey("cand-12345")
	if key != "claude:v1:cand-12345" {
		t.Fatalf("unexpected remember key: %q", key)
	}

	genericAdapter, ok := LookupApprovalAdapter("")
	if !ok {
		t.Fatal("generic adapter fallback failed")
	}
	genericKey := genericAdapter.RememberKey("cand-999")
	if genericKey != "generic:v1:cand-999" {
		t.Fatalf("unexpected generic remember key: %q", genericKey)
	}
}

func TestApprovalSummarizerMasksSecrets(t *testing.T) {
	adapter, _ := LookupApprovalAdapter("approval:claude-v1")
	summarizer := adapter.Summarizer()

	rawText := "curl -H 'Authorization: Bearer my-secret-token-12345678' https://api.example.com?api_key=supersecretpass123 with sk-proj-12345678901234567890"
	summary := summarizer.Summarize("Execute command?", rawText)

	if strings.Contains(summary.Command, "my-secret-token-12345678") || strings.Contains(summary.Raw, "my-secret-token-12345678") {
		t.Fatalf("Bearer token was not masked: %#v", summary)
	}
	if strings.Contains(summary.Command, "supersecretpass123") || strings.Contains(summary.Raw, "supersecretpass123") {
		t.Fatalf("API key was not masked: %#v", summary)
	}
	if strings.Contains(summary.Command, "sk-proj-12345678901234567890") || strings.Contains(summary.Raw, "sk-proj-12345678901234567890") {
		t.Fatalf("sk- token was not masked: %#v", summary)
	}

	if !strings.Contains(summary.Raw, "[REDACTED]") {
		t.Fatalf("expected [REDACTED] in raw summary, got: %s", summary.Raw)
	}
}

func TestClaudeDetectorFeedbackCard(t *testing.T) {
	adapter, _ := LookupApprovalAdapter("approval:claude-v1")
	detector := adapter.Detector()

	feedbackOptions := []proto.ApprovalOption{
		{Num: 1, Label: "1 to review"},
		{Num: 2, Label: "2 to send"},
	}
	if !detector.IsFeedbackCard(feedbackOptions) {
		t.Fatal("expected feedback card to be recognized for Claude")
	}

	normalOptions := []proto.ApprovalOption{
		{Num: 1, Label: "Yes"},
		{Num: 2, Label: "No"},
	}
	if detector.IsFeedbackCard(normalOptions) {
		t.Fatal("normal options should not be feedback card")
	}
}

func TestCodexDetectorNativeShortcut(t *testing.T) {
	adapter, _ := LookupApprovalAdapter("approval:codex-v1")
	detector := adapter.Detector()

	sendTextOptions := []proto.ApprovalOption{
		{Num: 1, Label: "Proceed", SendText: "y"},
	}

	matched, kind := detector.IsNativeShortcut("(P)", sendTextOptions)
	if !matched || kind != "native_codex_shortcut" {
		t.Fatalf("expected native codex shortcut match, got %v, %q", matched, kind)
	}

	matchedNoSendText, _ := detector.IsNativeShortcut("(P)", []proto.ApprovalOption{{Num: 1, Label: "Proceed"}})
	if matchedNoSendText {
		t.Fatal("should not match native shortcut without send text")
	}
}

func TestApprovalSenderFormat(t *testing.T) {
	adapter, _ := LookupApprovalAdapter("approval:generic-v1")
	sender := adapter.Sender()

	resNum := sender.FormatSend(proto.ApprovalOption{Num: 1})
	if resNum != "1\n" {
		t.Fatalf("expected '1\\n', got %q", resNum)
	}

	resSendText := sender.FormatSend(proto.ApprovalOption{Num: 2, SendText: "custom"})
	if resSendText != "custom\n" {
		t.Fatalf("expected 'custom\\n', got %q", resSendText)
	}
}

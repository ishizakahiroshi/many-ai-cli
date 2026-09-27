package provider

import "testing"

// compatibilityFixture is the C1 contract for values that must survive the
// registry migration. The fixture deliberately records unknown adapter
// details as "unknown" instead of turning an observed provider name into a
// capability claim.
type compatibilityFixture struct {
	ID                   string
	DisplayName          string
	ExecutableCandidates []string
	BaseArgs             []string
	ModelArgs            []string
	EffortArgs           []string
	Headless             string
	ApprovalAdapter      string
	TranscriptAdapter    string
	UsageAdapter         string
	SubscriptionAdapter  string
	Shell                bool
}

var compatibilityFixtures = []compatibilityFixture{
	{
		ID:                   "claude",
		DisplayName:          "Claude",
		ExecutableCandidates: []string{"claude"},
		ModelArgs:            []string{"--model", "{model}"},
		EffortArgs:           []string{"--effort", "{effort}"},
		Headless:             "claude-stream-json",
		ApprovalAdapter:      "claude-vt",
		TranscriptAdapter:    "claude-transcript",
		UsageAdapter:         "claude",
		SubscriptionAdapter:  "claude",
	},
	{
		ID:                   "codex",
		DisplayName:          "Codex",
		ExecutableCandidates: []string{"codex"},
		ModelArgs:            []string{"--model", "{model}"},
		EffortArgs:           []string{"-c", "model_reasoning_effort=\"{effort}\""},
		Headless:             "unknown",
		ApprovalAdapter:      "codex-vt",
		TranscriptAdapter:    "codex-transcript",
		UsageAdapter:         "codex",
		SubscriptionAdapter:  "codex",
	},
	{
		ID:                   "copilot",
		DisplayName:          "GitHub Copilot",
		ExecutableCandidates: []string{"copilot", "gh"},
		ModelArgs:            []string{"--model", "{model}"},
		Headless:             "text",
		ApprovalAdapter:      "copilot-vt",
		TranscriptAdapter:    "unknown",
		UsageAdapter:         "unknown",
		SubscriptionAdapter:  "unknown",
	},
	{
		ID:                   "cursor-agent",
		DisplayName:          "Cursor Agent",
		ExecutableCandidates: []string{"cursor-agent"},
		ModelArgs:            []string{"--model", "{model}"},
		Headless:             "text",
		ApprovalAdapter:      "cursor-agent-vt",
		TranscriptAdapter:    "cursor-agent",
		UsageAdapter:         "unknown",
		SubscriptionAdapter:  "unknown",
	},
	{
		ID:                   "opencode",
		DisplayName:          "OpenCode",
		ExecutableCandidates: []string{"opencode"},
		ModelArgs:            []string{"--model", "{model}"},
		EffortArgs:           []string{"--variant", "{effort}"},
		Headless:             "text",
		ApprovalAdapter:      "opencode-vt",
		TranscriptAdapter:    "opencode",
		UsageAdapter:         "opencode",
		SubscriptionAdapter:  "opencode",
	},
	{
		ID:                   "grok",
		DisplayName:          "Grok Build",
		ExecutableCandidates: []string{"grok"},
		ModelArgs:            []string{"--model", "{model}"},
		Headless:             "text",
		ApprovalAdapter:      "grok-vt",
		TranscriptAdapter:    "unknown",
		UsageAdapter:         "unknown",
		SubscriptionAdapter:  "grok",
	},
	{
		ID:                   "command-code",
		DisplayName:          "Command Code",
		ExecutableCandidates: []string{"command-code"},
		ModelArgs:            []string{"--model", "{model}"},
		Headless:             "text",
		ApprovalAdapter:      "command-code-vt",
		TranscriptAdapter:    "command-code",
		UsageAdapter:         "unknown",
		SubscriptionAdapter:  "unknown",
	},
	{
		ID:                   "custom",
		DisplayName:          "user-defined",
		ExecutableCandidates: []string{"<command[0]>"},
		BaseArgs:             []string{"<command[1:]>"},
		Headless:             "declared-by-config",
		ApprovalAdapter:      "generic-vt",
		TranscriptAdapter:    "unsupported",
		UsageAdapter:         "unsupported",
		SubscriptionAdapter:  "unsupported",
	},
	{
		ID:                   "shell",
		DisplayName:          "Shell",
		ExecutableCandidates: []string{"platform-default-shell"},
		Shell:                true,
		ApprovalAdapter:      "unsupported",
		TranscriptAdapter:    "unsupported",
		UsageAdapter:         "unsupported",
		SubscriptionAdapter:  "unsupported",
	},
}

func TestCompatibilityFixturesCoverCurrentProviderEntrypoints(t *testing.T) {
	want := []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code", "custom", "shell"}
	if len(compatibilityFixtures) != len(want) {
		t.Fatalf("compatibility fixture count = %d, want %d", len(compatibilityFixtures), len(want))
	}
	for i, fixture := range compatibilityFixtures {
		if fixture.ID != want[i] {
			t.Fatalf("compatibility fixture %d = %q, want %q", i, fixture.ID, want[i])
		}
		if fixture.DisplayName == "" || len(fixture.ExecutableCandidates) == 0 {
			t.Fatalf("compatibility fixture %q is missing display or executable candidates", fixture.ID)
		}
		for _, value := range append(append(append([]string{}, fixture.ExecutableCandidates...), fixture.BaseArgs...), fixture.ModelArgs...) {
			if value == "" {
				t.Fatalf("compatibility fixture %q contains an empty argv value", fixture.ID)
			}
		}
	}
}

func TestCompatibilityFixturesDoNotContainSecretsOrMachinePaths(t *testing.T) {
	for _, fixture := range compatibilityFixtures {
		for _, value := range append(append(append(append([]string{}, fixture.ExecutableCandidates...), fixture.BaseArgs...), fixture.ModelArgs...), fixture.EffortArgs...) {
			for _, forbidden := range []string{"token", "password", "secret", "C:\\Users\\", "/Users/"} {
				if containsFold(value, forbidden) {
					t.Fatalf("compatibility fixture %q contains forbidden marker %q", fixture.ID, forbidden)
				}
			}
		}
	}
}

func containsFold(value, needle string) bool {
	for i := 0; i+len(needle) <= len(value); i++ {
		match := true
		for j := range needle {
			if lowerASCII(value[i+j]) != lowerASCII(needle[j]) {
				match = false
				break
			}
		}
		if match {
			return true
		}
	}
	return false
}

func lowerASCII(b byte) byte {
	if b >= 'A' && b <= 'Z' {
		return b + ('a' - 'A')
	}
	return b
}

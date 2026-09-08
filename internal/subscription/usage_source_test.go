package subscription

import "testing"

// TestUsageSourceForUnknownProviderIsNone is the C6 completion criterion
// (子 plan: docs/local/plan_session-handoff-board_c6_usage-dispatch.md 内部
// C1): a provider not in the table must come back as UsageSourceNone, not a
// zero-value struct that happens to look like something else, and must not
// be treated as a handoff target.
func TestUsageSourceForUnknownProviderIsNone(t *testing.T) {
	got := UsageSourceFor("some-future-provider")
	if got.Kind != UsageSourceNone {
		t.Fatalf("Kind = %q, want %q", got.Kind, UsageSourceNone)
	}
	if got.CanDetectApproachingLimit {
		t.Fatalf("unknown provider must not be detectable: %#v", got)
	}
	if got.CanBeHandoffTarget {
		t.Fatalf("unknown provider must not be a handoff target: %#v", got)
	}
}

func TestUsageSourceForKnownProviders(t *testing.T) {
	cases := []struct {
		provider   string
		wantKind   UsageSourceKind
		wantDetect bool
		wantTarget bool
	}{
		{"claude", UsageSourcePushed, true, true},
		{"codex", UsageSourceLocalFile, true, true},
		{"grok", UsageSourceLocalFile, true, true},
		{"copilot", UsageSourceNone, false, true},
		{"cursor-agent", UsageSourceNone, false, true},
		{"opencode", UsageSourceNone, false, true},
	}
	for _, tc := range cases {
		t.Run(tc.provider, func(t *testing.T) {
			got := UsageSourceFor(tc.provider)
			if got.Kind != tc.wantKind {
				t.Fatalf("Kind = %q, want %q", got.Kind, tc.wantKind)
			}
			if got.CanDetectApproachingLimit != tc.wantDetect {
				t.Fatalf("CanDetectApproachingLimit = %v, want %v", got.CanDetectApproachingLimit, tc.wantDetect)
			}
			if got.CanBeHandoffTarget != tc.wantTarget {
				t.Fatalf("CanBeHandoffTarget = %v, want %v", got.CanBeHandoffTarget, tc.wantTarget)
			}
		})
	}
}

func TestLocalFileUsageProviders(t *testing.T) {
	got := LocalFileUsageProviders()
	want := map[string]bool{"codex": true, "grok": true}
	if len(got) != len(want) {
		t.Fatalf("LocalFileUsageProviders() = %v, want exactly %v", got, want)
	}
	for _, p := range got {
		if !want[p] {
			t.Fatalf("unexpected local-file provider %q in %v", p, got)
		}
	}
}

// TestHandoffTargetProvidersExcludesSource pins the same behavior
// internal/hub's handoffCandidateProviders relies on: the source provider is
// never offered as its own successor, and every other table provider is.
func TestHandoffTargetProvidersExcludesSource(t *testing.T) {
	got := HandoffTargetProviders("claude")
	for _, p := range got {
		if p == "claude" {
			t.Fatalf("HandoffTargetProviders must exclude the source provider: %v", got)
		}
	}
	want := map[string]bool{"codex": true, "grok": true, "copilot": true, "cursor-agent": true, "opencode": true}
	for _, p := range got {
		delete(want, p)
	}
	if len(want) != 0 {
		t.Fatalf("HandoffTargetProviders missing: %v (got %v)", want, got)
	}
}

package subscription

import (
	"slices"
	"testing"
)

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
		{"command-code", UsageSourceNone, false, true},
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

func TestHandoffTargetProvidersIncludesAllTargets(t *testing.T) {
	got := HandoffTargetProviders()
	want := []string{"claude", "codex", "grok", "copilot", "cursor-agent", "opencode", "command-code"}
	if !slices.Equal(got, want) {
		t.Fatalf("HandoffTargetProviders() = %v, want %v", got, want)
	}
}

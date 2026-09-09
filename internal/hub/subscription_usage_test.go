package hub

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func TestSubscriptionUsageRefreshesCodexAndGrokProfiles(t *testing.T) {
	root := t.TempDir()
	codexDir := filepath.Join(root, "codex")
	grokDir := filepath.Join(root, "grok")
	if err := os.MkdirAll(filepath.Join(codexDir, "sessions", "2026", "08", "19"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(grokDir, "logs"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(codexDir, "sessions", "2026", "08", "19", "rollout-test.jsonl"), []byte(`{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":89,"window_minutes":10080,"resets_at":1787196957},"secondary":null}}}
`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(grokDir, "logs", "unified.jsonl"), []byte(`{"ts":"2026-08-20T02:31:39Z","msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":27,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-08-23T11:28:29Z"}}}}
`), 0o600); err != nil {
		t.Fatal(err)
	}

	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex":   {{ID: "plus-a", Name: "Codex A", ProfileDir: codexDir}},
		"grok":    {{ID: "grok-a", Name: "Grok A", ProfileDir: grokDir}},
		"copilot": {{ID: "copilot-a", Name: "No usage"}},
	}}
	store := newSubscriptionUsageStore()
	store.refreshLocal(cfg, root, time.Unix(1_800_000_000, 0))
	response := store.snapshot(cfg)
	if len(response.Providers) != 2 {
		t.Fatalf("providers=%#v, want only codex/grok", response.Providers)
	}
	if response.Providers[0].Provider != "codex" || response.Providers[0].Profiles[0].Codex == nil || response.Providers[0].Profiles[0].Codex.Primary.UsedPercent != 89 {
		t.Fatalf("codex response=%#v", response.Providers[0])
	}
	if response.Providers[0].Profiles[0].Codex.Primary.RemainingPercent != 11 {
		t.Fatalf("codex remaining=%v want 11", response.Providers[0].Profiles[0].Codex.Primary.RemainingPercent)
	}
	grok := response.Providers[1].Profiles[0]
	if grok.Grok == nil || grok.Grok.UsedPercent != 27 {
		t.Fatalf("grok response=%#v", response.Providers[1])
	}
	gotRetrieved, err := time.Parse(time.RFC3339, grok.RetrievedAt)
	if err != nil {
		t.Fatalf("retrieved_at=%q: %v", grok.RetrievedAt, err)
	}
	wantRetrieved := time.Date(2026, 8, 20, 2, 31, 39, 0, time.UTC)
	if !gotRetrieved.Equal(wantRetrieved) {
		t.Fatalf("retrieved_at=%v want billing ts %v, not hub now", gotRetrieved, wantRetrieved)
	}
}

func TestSubscriptionUsagePresenceCanonicalAndExplicitClear(t *testing.T) {
	store := newSubscriptionUsageStore()
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude": {{ID: "claude-a", Name: "Claude A"}},
		"codex":  {{ID: "codex-a", Name: "Codex A"}},
	}}
	observed := time.Date(2026, 8, 26, 12, 0, 0, 0, time.UTC)
	store.recordSession("codex", "codex-a", &usageStat{
		CodexRateLimitsPresent:      true,
		CodexPrimaryPresent:         true,
		CodexPrimaryUsedPct:         37,
		CodexPrimaryWindowMinutes:   300,
		CodexSecondaryPresent:       true,
		CodexSecondaryUsedPct:       63,
		CodexSecondaryWindowMinutes: 10080,
		CodexCreditsPresent:         true,
		CodexHasCredits:             true,
		CodexCreditsBalance:         "0",
	}, observed)
	store.recordSession("claude", "claude-a", &usageStat{
		ClaudeRateLimitsPresent: true,
		ClaudeFiveHourPresent:   true,
		RateLimit5hPct:          0,
	}, observed)

	response := store.snapshot(cfg)
	var claude, codex *subscriptionUsageProfile
	for i := range response.Providers {
		for j := range response.Providers[i].Profiles {
			profile := &response.Providers[i].Profiles[j]
			switch profile.ID {
			case "claude-a":
				claude = profile
			case "codex-a":
				codex = profile
			}
		}
	}
	if claude == nil || claude.Claude == nil || claude.Claude.FiveHour == nil || claude.Claude.FiveHour.UsedPercent != 0 || claude.Claude.FiveHour.RemainingPercent != 100 || claude.Claude.SevenDay != nil {
		t.Fatalf("Claude presence/zero state = %#v", claude)
	}
	if codex == nil || codex.Codex == nil || codex.Codex.Primary == nil || codex.Codex.Secondary == nil {
		t.Fatalf("Codex windows = %#v", codex)
	}
	if codex.Codex.Primary.RemainingPercent != 63 || codex.Codex.Secondary.RemainingPercent != 37 || codex.Codex.Credits == nil || codex.Codex.Credits.Balance != "0" {
		t.Fatalf("Codex canonical/credits = %#v", codex.Codex)
	}
	if codex.RetrievedAt != observed.Format(time.RFC3339) {
		t.Fatalf("retrieved_at=%q want %q", codex.RetrievedAt, observed.Format(time.RFC3339))
	}

	if got := usageWindowFromSource(63, "remaining", 300, 0); got == nil || got.RemainingPercent != 63 || got.UsedPercent != 37 {
		t.Fatalf("remaining source normalization = %#v", got)
	}
	if got := usageWindowFromSource(63, "unknown", 300, 0); got != nil {
		t.Fatalf("unknown semantics produced a meter: %#v", got)
	}

	// An explicit empty observation replaces the previous two windows instead
	// of leaving a stale primary/secondary pair in the API.
	store.recordSession("codex", "codex-a", &usageStat{CodexRateLimitsPresent: true}, observed.Add(time.Minute))
	cleared := store.snapshot(cfg)
	for _, provider := range cleared.Providers {
		for _, profile := range provider.Profiles {
			if profile.ID == "codex-a" && (profile.Codex == nil || profile.Codex.Primary != nil || profile.Codex.Secondary != nil) {
				t.Fatalf("explicit empty observation retained old windows: %#v", profile.Codex)
			}
		}
	}
}

func TestSubscriptionUsageLiveValueSurvivesOfflineRefresh(t *testing.T) {
	store := newSubscriptionUsageStore()
	store.putLocal("codex", "plus-a", subscriptionUsageValue{
		codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
	}, time.Unix(10, 0))
	store.recordSession("codex", "plus-a", &usageStat{
		CodexRateLimitsPresent:    true,
		CodexPrimaryUsedPct:       55,
		CodexPrimaryWindowMinutes: 10080,
	}, time.Unix(20, 0))
	store.putLocal("codex", "plus-a", subscriptionUsageValue{
		codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
	}, time.Unix(30, 0))

	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex": {{ID: "plus-a", Name: "Codex A"}},
	}}
	row := store.snapshot(cfg).Providers[0].Profiles[0]
	if row.Codex == nil || row.Codex.Primary == nil || row.Codex.Primary.UsedPercent != 55 {
		t.Fatalf("live value was overwritten: %#v", row.Codex)
	}
}

func TestSubscriptionUsageUnacquiredIsNotZero(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex": {{ID: "never-used", Name: "Never used"}},
	}}
	row := newSubscriptionUsageStore().snapshot(cfg).Providers[0].Profiles[0]
	if row.Codex != nil || row.RetrievedAt != "" {
		t.Fatalf("unacquired profile was encoded as a value: %#v", row)
	}
}

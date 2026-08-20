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

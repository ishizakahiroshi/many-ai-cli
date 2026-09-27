package hub

import (
	"math"
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

// 上限到達後に 5h / 7d の使用率が 100 を超えて届いても、Hub は残量 100% に
// 化けさせず「使い切り」として扱う。clampPct のように範囲外を 0 へ丸めると、
// 上限に達したセッションほど残量表示と引き継ぎ帯が黙る。入力は合成データ。
func TestClampRateLimitUsedPctSaturatesInsteadOfZeroing(t *testing.T) {
	nan := math.NaN()
	cases := []struct {
		in, want float64
	}{
		{0, 0}, {42.5, 42.5}, {100, 100},
		{100.4, 100}, {105.5, 100}, {1e9, 100}, {math.Inf(1), 100},
		{-3, 0}, {math.Inf(-1), 0}, {nan, 0},
	}
	for _, tc := range cases {
		if got := clampRateLimitUsedPct(tc.in); got != tc.want {
			t.Errorf("clampRateLimitUsedPct(%v) = %v, want %v", tc.in, got, tc.want)
		}
	}

	store := newSubscriptionUsageStore()
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude": {{ID: "claude-a", Name: "Claude A"}},
	}}
	store.recordSession("claude", "claude-a", &usageStat{
		ClaudeRateLimitsPresent: true,
		ClaudeFiveHourPresent:   true,
		ClaudeSevenDayPresent:   true,
		RateLimit5hPct:          clampRateLimitUsedPct(105.5),
		RateLimit7dPct:          clampRateLimitUsedPct(41.2),
	}, time.Date(2026, 9, 26, 12, 0, 0, 0, time.UTC))
	for _, provider := range store.snapshot(cfg).Providers {
		for _, profile := range provider.Profiles {
			if profile.ID != "claude-a" {
				continue
			}
			c := profile.Claude
			if c == nil || c.FiveHour == nil || c.SevenDay == nil {
				t.Fatalf("claude windows = %#v", c)
			}
			if c.FiveHour.RemainingPercent != 0 || c.FiveHour.UsedPercent != 100 {
				t.Errorf("5h over 100 = %#v, want used 100 / remaining 0", c.FiveHour)
			}
			if c.SevenDay.UsedPercent != 41.2 {
				t.Errorf("7d = %#v, want used 41.2", c.SevenDay)
			}
			return
		}
	}
	t.Fatal("claude-a profile missing from snapshot")
}

func TestSubscriptionUsageObservationRecencyNotSourcePriority(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex": {{ID: "plus-a", Name: "Codex A"}},
	}}
	older := time.Unix(10, 0)
	newer := time.Unix(20, 0)
	latest := time.Unix(30, 0)

	t.Run("newer local supersedes older live", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.recordSession("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, older)
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 51, RemainingPercent: 49, WindowMinutes: 300}, Secondary: &usageWindow{UsedPercent: 69, RemainingPercent: 31, WindowMinutes: 10080}},
		}, newer)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 51 || got.Secondary == nil || got.Secondary.UsedPercent != 69 {
			t.Fatalf("newer local did not replace older empty live: %#v", got)
		}
	})

	t.Run("older local does not supersede newer live", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
		}, older)
		store.recordSession("codex", "plus-a", &usageStat{
			CodexRateLimitsPresent:    true,
			CodexPrimaryUsedPct:       55,
			CodexPrimaryWindowMinutes: 10080,
		}, newer)
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
		}, older)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 55 {
			t.Fatalf("older local replaced newer live: %#v", got)
		}
	})

	t.Run("delayed older live does not regress newer local", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 51, RemainingPercent: 49, WindowMinutes: 300}, Secondary: &usageWindow{UsedPercent: 69, RemainingPercent: 31, WindowMinutes: 10080}},
		}, newer)
		store.recordSession("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, older)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 51 || got.Secondary == nil || got.Secondary.UsedPercent != 69 {
			t.Fatalf("delayed empty live cleared newer local: %#v", got)
		}
	})

	t.Run("explicit newer empty local clears stale windows", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.recordSession("codex", "plus-a", &usageStat{
			CodexRateLimitsPresent:      true,
			CodexPrimaryUsedPct:         51,
			CodexPrimaryWindowMinutes:   300,
			CodexSecondaryUsedPct:       69,
			CodexSecondaryWindowMinutes: 10080,
		}, older)
		store.putLocal("codex", "plus-a", subscriptionUsageValue{codex: &codexSubscriptionUsage{}}, newer)
		got := snapshotCodex(t, store, cfg)
		if got.Primary != nil || got.Secondary != nil {
			t.Fatalf("newer empty local retained stale windows: %#v", got)
		}
	})

	t.Run("equal timestamps keep current", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.recordSession("codex", "plus-a", &usageStat{
			CodexRateLimitsPresent:    true,
			CodexPrimaryUsedPct:       55,
			CodexPrimaryWindowMinutes: 10080,
		}, newer)
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
		}, newer)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 55 {
			t.Fatalf("equal-time local replaced live: %#v", got)
		}
	})

	t.Run("absent incoming timestamp does not replace dated current", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.recordSession("codex", "plus-a", &usageStat{
			CodexRateLimitsPresent:    true,
			CodexPrimaryUsedPct:       55,
			CodexPrimaryWindowMinutes: 10080,
		}, newer)
		store.putObservation("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 89, WindowMinutes: 10080}},
		}, time.Time{}, latest, "local")
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 55 {
			t.Fatalf("undated local replaced dated live: %#v", got)
		}
	})

	t.Run("dated incoming replaces undated current", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.putObservation("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 12, WindowMinutes: 300}},
		}, time.Time{}, latest, "live")
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 51, RemainingPercent: 49, WindowMinutes: 300}},
		}, older)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 51 {
			t.Fatalf("dated local did not replace undated live: %#v", got)
		}
	})

	t.Run("undated delayed live cannot replace dated local", func(t *testing.T) {
		store := newSubscriptionUsageStore()
		store.putLocal("codex", "plus-a", subscriptionUsageValue{
			codex: &codexSubscriptionUsage{Primary: &usageWindow{UsedPercent: 51}},
		}, older)
		store.recordSessionAt("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, time.Time{}, latest)
		got := snapshotCodex(t, store, cfg)
		if got.Primary == nil || got.Primary.UsedPercent != 51 {
			t.Fatalf("undated live replaced dated local: %#v", got)
		}
	})
}

func TestSubscriptionUsageMissingCodexRateLimitsDoesNotClear(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex": {{ID: "plus-a", Name: "Codex A"}},
	}}
	observed := time.Unix(20, 0)
	store := newSubscriptionUsageStore()
	store.recordSession("codex", "plus-a", &usageStat{
		CodexRateLimitsPresent:      true,
		CodexPrimaryUsedPct:         51,
		CodexPrimaryWindowMinutes:   300,
		CodexSecondaryUsedPct:       69,
		CodexSecondaryWindowMinutes: 10080,
	}, observed)
	store.recordSession("codex", "plus-a", &usageStat{TokensIn: 3}, observed.Add(time.Minute))
	got := snapshotCodex(t, store, cfg)
	if got.Primary == nil || got.Primary.UsedPercent != 51 || got.Secondary == nil || got.Secondary.UsedPercent != 69 {
		t.Fatalf("missing rate_limits cleared cache: %#v", got)
	}
}

func TestSubscriptionUsageCodexRefreshLocalNewerThanLive(t *testing.T) {
	root := t.TempDir()
	codexDir := filepath.Join(root, "codex")
	if err := os.MkdirAll(filepath.Join(codexDir, "sessions", "2026", "09", "19"), 0o700); err != nil {
		t.Fatal(err)
	}
	body := `{"timestamp":"2026-09-19T09:30:50.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":51,"window_minutes":300,"resets_at":1789800000},"secondary":{"used_percent":69,"window_minutes":10080,"resets_at":1789900000}}}}
`
	if err := os.WriteFile(filepath.Join(codexDir, "sessions", "2026", "09", "19", "rollout-synthetic.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"codex": {{ID: "plus-a", Name: "Codex A", ProfileDir: codexDir}},
	}}
	store := newSubscriptionUsageStore()
	liveAt := time.Date(2026, 9, 19, 5, 36, 28, 0, time.UTC)
	store.recordSession("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, liveAt)
	store.refreshLocal(cfg, root, time.Date(2026, 9, 19, 9, 31, 0, 0, time.UTC))
	got := snapshotCodex(t, store, cfg)
	if got.Primary == nil || got.Primary.UsedPercent != 51 || got.Secondary == nil || got.Secondary.UsedPercent != 69 {
		t.Fatalf("refreshLocal did not apply newer dated rollout: %#v", got)
	}

	if err := os.WriteFile(filepath.Join(codexDir, "sessions", "2026", "09", "19", "rollout-synthetic.jsonl"), []byte(`{"timestamp":"2026-09-19T09:32:00.000Z","type":"event_msg","payload":{"type":"token_count"}}
`), 0o600); err != nil {
		t.Fatal(err)
	}
	store.refreshLocal(cfg, root, time.Date(2026, 9, 19, 9, 33, 0, 0, time.UTC))
	got = snapshotCodex(t, store, cfg)
	if got.Primary == nil || got.Primary.UsedPercent != 51 || got.Secondary == nil || got.Secondary.UsedPercent != 69 {
		t.Fatalf("token_count without rate_limits cleared cache: %#v", got)
	}

	store.recordSession("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, liveAt)
	got = snapshotCodex(t, store, cfg)
	if got.Primary == nil || got.Primary.UsedPercent != 51 {
		t.Fatalf("delayed older stop hook regressed newer local: %#v", got)
	}
}

func TestSubscriptionUsageClaudeAndGrokStayIndependent(t *testing.T) {
	root := t.TempDir()
	grokDir := filepath.Join(root, "grok")
	if err := os.MkdirAll(filepath.Join(grokDir, "logs"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(grokDir, "logs", "unified.jsonl"), []byte(`{"ts":"2026-08-20T02:31:39Z","msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":27,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-08-23T11:28:29Z"}}}}
`), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude": {{ID: "claude-a", Name: "Claude A"}},
		"grok":   {{ID: "grok-a", Name: "Grok A", ProfileDir: grokDir}},
		"codex":  {{ID: "plus-a", Name: "Codex A"}},
	}}
	store := newSubscriptionUsageStore()
	claudeAt := time.Date(2026, 8, 20, 1, 0, 0, 0, time.UTC)
	store.recordSession("claude", "claude-a", &usageStat{
		ClaudeRateLimitsPresent: true,
		ClaudeFiveHourPresent:   true,
		RateLimit5hPct:          12,
	}, claudeAt)
	store.recordSession("claude", "claude-a", &usageStat{
		ClaudeRateLimitsPresent: true,
		ClaudeFiveHourPresent:   true,
		RateLimit5hPct:          18,
	}, claudeAt)
	store.recordSession("codex", "plus-a", &usageStat{CodexRateLimitsPresent: true}, claudeAt)
	store.refreshLocal(cfg, root, time.Date(2026, 8, 20, 3, 0, 0, 0, time.UTC))
	if err := os.WriteFile(filepath.Join(grokDir, "logs", "unified.jsonl"), []byte(`{"ts":"2026-08-20T02:31:39Z","msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":32,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-08-23T11:28:29Z"}}}}
`), 0o600); err != nil {
		t.Fatal(err)
	}
	store.refreshLocal(cfg, root, time.Date(2026, 8, 20, 3, 1, 0, 0, time.UTC))

	response := store.snapshot(cfg)
	var claude, grok *subscriptionUsageProfile
	for i := range response.Providers {
		for j := range response.Providers[i].Profiles {
			profile := &response.Providers[i].Profiles[j]
			switch profile.ID {
			case "claude-a":
				claude = profile
			case "grok-a":
				grok = profile
			}
		}
	}
	if claude == nil || claude.Claude == nil || claude.Claude.FiveHour == nil || claude.Claude.FiveHour.UsedPercent != 18 {
		t.Fatalf("claude usage changed by codex recency path: %#v", claude)
	}
	if grok == nil || grok.Grok == nil || grok.Grok.UsedPercent != 32 {
		t.Fatalf("grok local refresh lost: %#v", grok)
	}
	gotRetrieved, err := time.Parse(time.RFC3339, grok.RetrievedAt)
	if err != nil {
		t.Fatalf("retrieved_at=%q: %v", grok.RetrievedAt, err)
	}
	wantRetrieved := time.Date(2026, 8, 20, 2, 31, 39, 0, time.UTC)
	if !gotRetrieved.Equal(wantRetrieved) {
		t.Fatalf("grok retrieved_at=%v want billing ts %v", gotRetrieved, wantRetrieved)
	}
}

func snapshotCodex(t *testing.T, store *subscriptionUsageStore, cfg *config.Config) *codexSubscriptionUsage {
	t.Helper()
	row := store.snapshot(cfg).Providers[0].Profiles[0]
	if row.Codex == nil {
		t.Fatalf("missing codex snapshot: %#v", row)
	}
	return row.Codex
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

package usagelocal

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestReadCodexProfileReadsLatestRateLimits(t *testing.T) {
	root := filepath.Join(t.TempDir(), "sessions", "2026", "08", "19")
	if err := os.MkdirAll(root, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, "rollout-2026-08-19.jsonl")
	body := `{"type":"response_item","payload":{"type":"message","content":"must not be surfaced"}}
 {"timestamp":"2026-08-26T12:34:56.123Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":89,"window_minutes":10080,"resets_at":1787196957},"secondary":null,"credits":{"has_credits":false,"unlimited":false,"balance":"0"},"plan_type":"plus"}}}
`
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	usage, ok := ReadCodexProfile(filepath.Dir(filepath.Dir(filepath.Dir(filepath.Dir(root)))))
	if !ok || usage.Primary == nil || usage.Primary.UsedPercent != 89 || usage.Primary.WindowMinutes != 10080 {
		t.Fatalf("usage=%#v ok=%v", usage, ok)
	}
	if usage.Secondary != nil || usage.CreditsBalance != "" {
		t.Fatalf("null/empty credits were interpreted as values: %#v", usage)
	}
	if !usage.RateLimitsPresent || usage.ObservedAt.IsZero() {
		t.Fatalf("presence/timestamp lost: %#v", usage)
	}
}

func TestReadCodexProfileDistinguishesPresenceAndCredits(t *testing.T) {
	cases := []struct {
		name           string
		rateLimits     string
		wantOK         bool
		wantPresent    bool
		wantPrimary    bool
		wantUsed       float64
		wantCredits    bool
		wantHasCredits bool
		wantUnlimited  bool
		wantBalance    string
	}{
		{
			name:           "zero percent is present",
			rateLimits:     `{"primary":{"used_percent":0,"window_minutes":300,"resets_at":1787196957},"secondary":null,"credits":{"has_credits":true,"unlimited":false,"balance":"0"},"plan_type":"pro"}`,
			wantOK:         true,
			wantPresent:    true,
			wantPrimary:    true,
			wantCredits:    true,
			wantHasCredits: true,
			wantBalance:    "0",
		},
		{
			name:        "explicit null clears",
			rateLimits:  `null`,
			wantOK:      true,
			wantPresent: true,
		},
		{
			name:       "missing field is not an observation",
			rateLimits: "__missing__",
			wantOK:     true,
		},
		{
			name:        "credits absent",
			rateLimits:  `{"primary":null}`,
			wantOK:      true,
			wantPresent: true,
			wantCredits: false,
		},
		{
			name:        "credits unavailable",
			rateLimits:  `{"credits":{"has_credits":false,"unlimited":false,"balance":""}}`,
			wantOK:      true,
			wantPresent: true,
			wantCredits: true,
		},
		{
			name:           "credits unlimited",
			rateLimits:     `{"credits":{"has_credits":true,"unlimited":true,"balance":""}}`,
			wantOK:         true,
			wantPresent:    true,
			wantCredits:    true,
			wantHasCredits: true,
			wantUnlimited:  true,
		},
		{
			name:       "malformed auxiliary value does not fall back",
			rateLimits: `"not-an-object"`,
			wantOK:     false,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			profileDir := t.TempDir()
			rolloutDir := filepath.Join(profileDir, "sessions", "2026", "08", "26")
			if err := os.MkdirAll(rolloutDir, 0o700); err != nil {
				t.Fatal(err)
			}
			line := `{"timestamp":"2026-08-26T12:34:56.123Z","type":"event_msg","payload":{"type":"token_count"`
			if tc.rateLimits != "__missing__" {
				line += `,"rate_limits":` + tc.rateLimits
			}
			line += `}}` + "\n"
			if err := os.WriteFile(filepath.Join(rolloutDir, "rollout-presence.jsonl"), []byte(line), 0o600); err != nil {
				t.Fatal(err)
			}
			usage, ok := ReadCodexProfile(profileDir)
			if ok != tc.wantOK {
				t.Fatalf("ok=%v want %v usage=%#v", ok, tc.wantOK, usage)
			}
			if !ok {
				return
			}
			if usage.RateLimitsPresent != tc.wantPresent || (usage.Primary != nil) != tc.wantPrimary {
				t.Fatalf("presence/primary = %v/%v, want %v/%v: %#v", usage.RateLimitsPresent, usage.Primary != nil, tc.wantPresent, tc.wantPrimary, usage)
			}
			if usage.Primary != nil && usage.Primary.UsedPercent != tc.wantUsed {
				t.Fatalf("used_percent=%v want %v", usage.Primary.UsedPercent, tc.wantUsed)
			}
			if usage.Credits.Present != tc.wantCredits || usage.Credits.HasCredits != tc.wantHasCredits || usage.Credits.Unlimited != tc.wantUnlimited || usage.Credits.Balance != tc.wantBalance {
				t.Fatalf("credits=%#v", usage.Credits)
			}
		})
	}
}

func TestReadGrokProfileReadsLatestBillingRecord(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "logs")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	body := `{"msg":"other","ctx":{"config":{}}}
{"ts":"2026-08-20T02:31:39Z","msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":27,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-08-16T11:28:29Z","end":"2026-08-23T11:28:29Z"}}}}
`
	if err := os.WriteFile(filepath.Join(dir, "unified.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	usage, ok := ReadGrokProfile(filepath.Dir(dir))
	if !ok || usage.UsedPercent != 27 || usage.PeriodEnd == "" || usage.PeriodType == "" {
		t.Fatalf("usage=%#v ok=%v", usage, ok)
	}
	wantFetched := time.Date(2026, 8, 20, 2, 31, 39, 0, time.UTC)
	if !usage.FetchedAt.Equal(wantFetched) {
		t.Fatalf("FetchedAt=%v want %v", usage.FetchedAt, wantFetched)
	}
}

func TestReadGrokProfileSkipsRecordsWithoutPercent(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "logs")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	body := `{"ts":"2026-08-20T02:23:11Z","msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":2,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-08-21T00:15:49Z"}}}}
{"ts":"2026-08-20T02:28:15Z","msg":"billing: fetched credits config","ctx":{"config":{"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-08-21T00:15:49Z"}}}}
`
	if err := os.WriteFile(filepath.Join(dir, "unified.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	usage, ok := ReadGrokProfile(filepath.Dir(dir))
	if !ok || usage.UsedPercent != 2 {
		t.Fatalf("usage=%#v ok=%v", usage, ok)
	}
	wantFetched := time.Date(2026, 8, 20, 2, 23, 11, 0, time.UTC)
	if !usage.FetchedAt.Equal(wantFetched) {
		t.Fatalf("FetchedAt=%v want %v", usage.FetchedAt, wantFetched)
	}
}

func TestReadLocalUsageMissingOrMalformedReturnsUnacquired(t *testing.T) {
	if _, ok := ReadCodexProfile(t.TempDir()); ok {
		t.Fatal("missing Codex profile reported as acquired")
	}
	dir := filepath.Join(t.TempDir(), "logs")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "unified.jsonl"), []byte("not json\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := ReadGrokProfile(filepath.Dir(dir)); ok {
		t.Fatal("malformed Grok record reported as acquired")
	}
}

func TestNewestRolloutPrefersNewestModification(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	old := filepath.Join(dir, "rollout-old.jsonl")
	newer := filepath.Join(dir, "rollout-new.jsonl")
	for _, path := range []string{old, newer} {
		if err := os.WriteFile(path, []byte("{}\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Chtimes(old, time.Unix(1, 0), time.Unix(1, 0)); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(newer, time.Unix(2, 0), time.Unix(2, 0)); err != nil {
		t.Fatal(err)
	}
	got, ok := newestRollout(dir)
	if !ok || got != newer {
		t.Fatalf("newestRollout=%q ok=%v, want %q", got, ok, newer)
	}
}

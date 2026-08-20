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
{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":89,"window_minutes":10080,"resets_at":1787196957},"secondary":null,"credits":{"has_credits":false,"unlimited":false,"balance":"0"},"plan_type":"plus"}}}
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

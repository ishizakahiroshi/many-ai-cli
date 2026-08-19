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
{"msg":"billing: fetched credits config","ctx":{"config":{"creditUsagePercent":27,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-08-16T11:28:29Z","end":"2026-08-23T11:28:29Z"}}}}
`
	if err := os.WriteFile(filepath.Join(dir, "unified.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	usage, ok := ReadGrokProfile(filepath.Dir(dir))
	if !ok || usage.UsedPercent != 27 || usage.PeriodEnd == "" || usage.PeriodType == "" {
		t.Fatalf("usage=%#v ok=%v", usage, ok)
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

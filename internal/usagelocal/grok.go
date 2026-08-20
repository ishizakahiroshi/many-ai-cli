package usagelocal

import (
	"encoding/json"
	"path/filepath"
	"strings"
	"time"
)

type grokBillingRecord struct {
	Ts  string `json:"ts"`
	Msg string `json:"msg"`
	Ctx struct {
		Config struct {
			CreditUsagePercent *float64 `json:"creditUsagePercent"`
			CurrentPeriod      struct {
				Start string `json:"start"`
				End   string `json:"end"`
				Type  string `json:"type"`
			} `json:"currentPeriod"`
		} `json:"config"`
	} `json:"ctx"`
}

func parseGrokTime(value string) time.Time {
	value = strings.TrimSpace(value)
	if value == "" {
		return time.Time{}
	}
	if t, err := time.Parse(time.RFC3339Nano, value); err == nil {
		return t
	}
	if t, err := time.Parse(time.RFC3339, value); err == nil {
		return t
	}
	return time.Time{}
}

// ReadGrokProfile returns the newest billing record in unified.jsonl. The
// file is a debug log whose shape can change, so malformed/missing records are
// intentionally treated as not acquired rather than surfaced as errors.
func ReadGrokProfile(profileDir string) (usage GrokUsage, ok bool) {
	path := filepath.Join(profileDir, "logs", "unified.jsonl")
	var found GrokUsage
	var foundRecord bool
	err := scanReverseLines(path, func(line []byte) bool {
		var record grokBillingRecord
		if json.Unmarshal(line, &record) != nil || record.Msg != "billing: fetched credits config" || record.Ctx.Config.CreditUsagePercent == nil {
			return false
		}
		found = GrokUsage{
			UsedPercent: *record.Ctx.Config.CreditUsagePercent,
			PeriodStart: record.Ctx.Config.CurrentPeriod.Start,
			PeriodEnd:   record.Ctx.Config.CurrentPeriod.End,
			PeriodType:  record.Ctx.Config.CurrentPeriod.Type,
			FetchedAt:   parseGrokTime(record.Ts),
		}
		foundRecord = true
		return true
	})
	if err != nil || !foundRecord {
		return GrokUsage{}, false
	}
	return found, true
}

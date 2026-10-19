package usagelocal

import (
	"bytes"
	"encoding/json"
	"io/fs"
	"path/filepath"
	"strings"
	"time"
)

type codexRateLimits struct {
	Primary   *RateLimitWindow `json:"primary"`
	Secondary *RateLimitWindow `json:"secondary"`
	Credits   *struct {
		HasCredits bool   `json:"has_credits"`
		Unlimited  bool   `json:"unlimited"`
		Balance    string `json:"balance"`
	} `json:"credits"`
	PlanType string `json:"plan_type"`
}

type codexRolloutEvent struct {
	Timestamp string `json:"timestamp"`
	Payload   struct {
		Type       string          `json:"type"`
		RateLimits json.RawMessage `json:"rate_limits"`
	} `json:"payload"`
}

// ReadCodexProfile returns the newest token_count observation under a
// profile's sessions directory. RateLimitsPresent distinguishes an explicit
// rate_limits value (including null/{}) from an older token_count record that
// predates the field. Missing files and malformed records are represented by
// ok=false.
func ReadCodexProfile(profileDir string) (usage CodexUsage, ok bool) {
	path, found := newestRollout(filepath.Join(profileDir, "sessions"))
	if !found {
		return CodexUsage{}, false
	}
	var foundUsage CodexUsage
	var foundRecord bool
	var malformedRecord bool
	err := scanReverseLines(path, func(line []byte) bool {
		if !bytes.Contains(line, []byte(`"token_count"`)) {
			return false
		}
		var event codexRolloutEvent
		if json.Unmarshal(line, &event) != nil {
			// A candidate token_count line that is not valid JSON is malformed;
			// do not fall back to an older record and resurrect stale limits.
			malformedRecord = true
			return true
		}
		if event.Payload.Type != "token_count" {
			return false
		}
		var limits codexRateLimits
		rateLimitsPresent := event.Payload.RateLimits != nil
		if len(event.Payload.RateLimits) > 0 && string(event.Payload.RateLimits) != "null" {
			if json.Unmarshal(event.Payload.RateLimits, &limits) != nil {
				// The token_count event itself is valid, but its auxiliary
				// rate_limits value is not. Keep the existing cache rather than
				// replacing it with a misleading partial record.
				malformedRecord = true
				return true
			}
		}
		foundUsage = CodexUsage{
			Primary:           limits.Primary,
			Secondary:         limits.Secondary,
			PlanType:          limits.PlanType,
			RateLimitsPresent: rateLimitsPresent,
			ObservedAt:        parseObservedAt(event.Timestamp),
		}
		if limits.Credits != nil {
			foundUsage.Credits = CreditsState{
				Present:    true,
				HasCredits: limits.Credits.HasCredits,
				Unlimited:  limits.Credits.Unlimited,
				Balance:    limits.Credits.Balance,
			}
			if limits.Credits.HasCredits && !limits.Credits.Unlimited {
				foundUsage.CreditsBalance = limits.Credits.Balance
			}
		}
		foundRecord = true
		return true
	})
	if err != nil || !foundRecord || malformedRecord {
		return CodexUsage{}, false
	}
	return foundUsage, true
}

func parseObservedAt(raw string) time.Time {
	if strings.TrimSpace(raw) == "" {
		return time.Time{}
	}
	parsed, err := time.Parse(time.RFC3339Nano, raw)
	if err != nil {
		return time.Time{}
	}
	return parsed
}

func newestRollout(root string) (string, bool) {
	var newest string
	var newestModTime int64
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() || !strings.HasPrefix(entry.Name(), "rollout-") || !strings.HasSuffix(entry.Name(), ".jsonl") {
			return nil
		}
		info, err := entry.Info()
		if err != nil {
			return nil
		}
		mod := info.ModTime().UnixNano()
		if newest == "" || mod > newestModTime || (mod == newestModTime && path > newest) {
			newest = path
			newestModTime = mod
		}
		return nil
	})
	return newest, err == nil && newest != ""
}

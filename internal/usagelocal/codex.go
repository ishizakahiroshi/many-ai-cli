package usagelocal

import (
	"bytes"
	"encoding/json"
	"io/fs"
	"path/filepath"
	"strings"
)

type codexRateLimits struct {
	Primary   *RateLimitWindow `json:"primary"`
	Secondary *RateLimitWindow `json:"secondary"`
	Credits   struct {
		HasCredits bool   `json:"has_credits"`
		Unlimited  bool   `json:"unlimited"`
		Balance    string `json:"balance"`
	} `json:"credits"`
	PlanType string `json:"plan_type"`
}

type codexRolloutEvent struct {
	Payload struct {
		Type       string           `json:"type"`
		RateLimits *codexRateLimits `json:"rate_limits"`
	} `json:"payload"`
}

// ReadCodexProfile returns the newest rate_limits record under a profile's
// sessions directory. Missing files, malformed records, and format changes
// are all represented by ok=false.
func ReadCodexProfile(profileDir string) (usage CodexUsage, ok bool) {
	path, found := newestRollout(filepath.Join(profileDir, "sessions"))
	if !found {
		return CodexUsage{}, false
	}
	var foundUsage CodexUsage
	var foundRecord bool
	err := scanReverseLines(path, func(line []byte) bool {
		if !bytes.Contains(line, []byte(`"token_count"`)) {
			return false
		}
		var event codexRolloutEvent
		if json.Unmarshal(line, &event) != nil || event.Payload.Type != "token_count" || event.Payload.RateLimits == nil {
			return false
		}
		limits := event.Payload.RateLimits
		foundUsage = CodexUsage{
			Primary:   limits.Primary,
			Secondary: limits.Secondary,
			PlanType:  limits.PlanType,
		}
		if limits.Credits.HasCredits && !limits.Credits.Unlimited {
			foundUsage.CreditsBalance = limits.Credits.Balance
		}
		foundRecord = true
		return true
	})
	if err != nil || !foundRecord {
		return CodexUsage{}, false
	}
	return foundUsage, true
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

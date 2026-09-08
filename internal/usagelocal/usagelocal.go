// Package usagelocal reads provider usage metadata already written to local
// CLI logs. It never opens authentication files and treats missing or changed
// log formats as an ordinary "not acquired" result.
package usagelocal

import (
	"bytes"
	"io"
	"os"
	"time"

	"many-ai-cli/internal/subscription"
)

// LocalProfile is the provider-agnostic result of ReadProfile: exactly one of
// Codex/Grok is set when a local-file usage record was found, and neither is
// when it was not.
type LocalProfile struct {
	Codex      *CodexUsage
	Grok       *GrokUsage
	ObservedAt time.Time
}

// ReadProfile is this package's single provider fan-out point (子 plan:
// docs/local/plan_session-handoff-board_c6_usage-dispatch.md 内部 C1
// 「usagelocal の呼び分けを、この表を引く形に置き換える」). It first asks
// subscription.UsageSourceFor whether provider even has a local file to read;
// a provider absent from that table, or present with a different Kind, never
// touches disk. The byte-level parsing for each supported provider stays in
// its own file (codex.go / grok.go) — that part is deliberately not
// table-driven, only the decision to attempt it is.
func ReadProfile(provider, profileDir string) (LocalProfile, bool) {
	if subscription.UsageSourceFor(provider).Kind != subscription.UsageSourceLocalFile {
		return LocalProfile{}, false
	}
	switch provider {
	case "codex":
		usage, ok := ReadCodexProfile(profileDir)
		if !ok || !usage.RateLimitsPresent {
			// Mirrors the caller's previous behavior: a token_count record
			// without an explicit rate_limits value is not a new observation,
			// so it must not overwrite a previously cached one.
			return LocalProfile{}, false
		}
		return LocalProfile{Codex: &usage, ObservedAt: usage.ObservedAt}, true
	case "grok":
		usage, ok := ReadGrokProfile(profileDir)
		if !ok {
			return LocalProfile{}, false
		}
		return LocalProfile{Grok: &usage, ObservedAt: usage.FetchedAt}, true
	default:
		// A provider can only reach here if usageSources and this switch
		// disagree about which providers are UsageSourceLocalFile — treated
		// as "not acquired" rather than a panic, matching every other
		// not-found path in this package.
		return LocalProfile{}, false
	}
}

// RateLimitWindow is one provider-reported usage window. A pointer to this
// type is used by callers so 0% remains distinguishable from an absent window.
type RateLimitWindow struct {
	UsedPercent   float64 `json:"used_percent"`
	WindowMinutes int     `json:"window_minutes"`
	ResetsAt      int64   `json:"resets_at"`
}

// CreditsState preserves the provider's credits presence and state without
// guessing a unit for balance.
type CreditsState struct {
	Present    bool
	HasCredits bool
	Unlimited  bool
	Balance    string
}

// CodexUsage is the subset of Codex rate_limits safe to show in the UI.
type CodexUsage struct {
	Primary   *RateLimitWindow
	Secondary *RateLimitWindow
	PlanType  string
	// RateLimitsPresent distinguishes a token_count record that explicitly
	// supplied rate_limits (including null or {}) from an older record that did
	// not have the field yet. The former may clear a cached observation; the
	// latter is not a new usage observation.
	RateLimitsPresent bool
	Credits           CreditsState
	// CreditsBalance remains for callers written against the pre-presence API.
	CreditsBalance string
	ObservedAt     time.Time
}

// GrokUsage is the weekly billing record written by Grok Build.
type GrokUsage struct {
	UsedPercent float64
	PeriodStart string
	PeriodEnd   string
	PeriodType  string
	// FetchedAt is the ts on that billing record, not the time Hub reread the file.
	FetchedAt time.Time
}

// scanReverseLines visits complete lines from the end of path toward the
// beginning. It intentionally does not read the complete file into memory:
// unified.jsonl and Codex rollouts grow without a fixed upper bound.
func scanReverseLines(path string, visit func([]byte) bool) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()

	info, err := f.Stat()
	if err != nil {
		return err
	}
	const chunkSize int64 = 64 * 1024
	end := info.Size()
	var pending []byte
	for end > 0 {
		start := end - chunkSize
		if start < 0 {
			start = 0
		}
		chunk := make([]byte, end-start)
		if _, err := f.ReadAt(chunk, start); err != nil && err != io.EOF {
			return err
		}
		data := make([]byte, 0, len(chunk)+len(pending))
		data = append(data, chunk...)
		data = append(data, pending...)

		cursor := len(data)
		for cursor > 0 {
			idx := bytes.LastIndexByte(data[:cursor], '\n')
			if idx < 0 {
				pending = append(pending[:0], data[:cursor]...)
				break
			}
			line := data[idx+1 : cursor]
			if len(line) > 0 && visit(line) {
				return nil
			}
			cursor = idx
			pending = nil
		}
		if cursor == 0 {
			pending = nil
		}
		end = start
	}
	if len(pending) > 0 {
		visit(pending)
	}
	return nil
}

// Package usagelocal reads provider usage metadata already written to local
// CLI logs. It never opens authentication files and treats missing or changed
// log formats as an ordinary "not acquired" result.
package usagelocal

import (
	"bytes"
	"io"
	"os"
)

// RateLimitWindow is one provider-reported usage window. A pointer to this
// type is used by callers so 0% remains distinguishable from an absent window.
type RateLimitWindow struct {
	UsedPercent   float64 `json:"used_percent"`
	WindowMinutes int     `json:"window_minutes"`
	ResetsAt      int64   `json:"resets_at"`
}

// CodexUsage is the subset of Codex rate_limits safe to show in the UI.
type CodexUsage struct {
	Primary        *RateLimitWindow
	Secondary      *RateLimitWindow
	PlanType       string
	CreditsBalance string
}

// GrokUsage is the weekly billing record written by Grok Build.
type GrokUsage struct {
	UsedPercent float64
	PeriodStart string
	PeriodEnd   string
	PeriodType  string
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

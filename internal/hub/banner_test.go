package hub

import (
	"strings"
	"testing"
)

func TestStartupBannerIncludesProductDetails(t *testing.T) {
	got := startupBanner("0.1.3", "127.0.0.1:47777", "abc123")

	for _, want := range []string{
		"Claude Code / Codex wrapper     v0.1.3",
		"GitHub: https://github.com/ishizakahiroshi/any-ai-cli",
		"WebUI:  http://127.0.0.1:47777",
		"Open:   http://127.0.0.1:47777/?token=abc123",
		"注意: この画面は Web UI と連結しています。閉じないでください。",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("startupBanner() missing %q in:\n%s", want, got)
		}
	}

	for _, want := range []string{ansiBold, ansiBlink, ansiBrightOrange, ansiReset} {
		if !strings.Contains(got, want) {
			t.Fatalf("startupBanner() missing ANSI code %q in:\n%s", want, got)
		}
	}
}

func TestFormatVersionLabel(t *testing.T) {
	tests := map[string]string{
		"":       "dev",
		"dev":    "dev",
		"0.1.3":  "v0.1.3",
		"v0.1.3": "v0.1.3",
	}

	for input, want := range tests {
		if got := formatVersionLabel(input); got != want {
			t.Fatalf("formatVersionLabel(%q) = %q, want %q", input, got, want)
		}
	}
}

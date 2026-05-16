package hub

import (
	"strings"
	"testing"
)

func TestStartupBannerIncludesProductDetails(t *testing.T) {
	t.Setenv("ANY_AI_CLI_WSL_LAUNCHER", "")
	got := startupBanner("0.1.3", "127.0.0.1:47777", "abc123")

	for _, want := range []string{
		"Claude Code / Codex wrapper     v0.1.3",
		"Runtime: ",
		"GitHub: https://github.com/ishizakahiroshi/any-ai-cli",
		"WebUI:  http://127.0.0.1:47777",
		"Open:   http://127.0.0.1:47777/?token=abc123",
		"WARNING: This window is connected to the Web UI. Do not close it.",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("startupBanner() missing %q in:\n%s", want, got)
		}
	}

	for _, want := range []string{ansiBold, ansiReverse, ansiBrightOrange, ansiReset} {
		if !strings.Contains(got, want) {
			t.Fatalf("startupBanner() missing ANSI code %q in:\n%s", want, got)
		}
	}

	// Default (non-launcher) variant must use the Unicode block-art logo so
	// terminals that render it cleanly (Linux/macOS/Windows Terminal) keep
	// the prettier banner.
	if !strings.Contains(got, "█") {
		t.Fatalf("startupBanner() unicode variant expected to contain U+2588 block char")
	}
}

func TestStartupBannerUsesAsciiUnderWindowsLauncher(t *testing.T) {
	// any-ai-cli-wsl.exe sets ANY_AI_CLI_WSL_LAUNCHER=1 in the WSL shell so
	// the Linux Hub knows its stdout is being rendered by conhost.exe, where
	// EAW=Ambiguous block / box-drawing chars are promoted to full-width and
	// distort the ASCII art. The banner must fall back to plain ASCII glyphs
	// (single-byte, unambiguous width) in that mode.
	t.Setenv("ANY_AI_CLI_WSL_LAUNCHER", "1")
	got := startupBanner("0.1.3", "127.0.0.1:47777", "abc123")

	for _, banned := range []string{"█", "╗", "╔", "╝", "╚", "║", "═"} {
		if strings.Contains(got, banned) {
			t.Fatalf("startupBanner() launcher variant must not contain %q", banned)
		}
	}

	// Product detail lines and the warning still ship — only the logo art
	// changes.
	for _, want := range []string{
		"Claude Code / Codex wrapper     v0.1.3",
		"WARNING: This window is connected to the Web UI. Do not close it.",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("startupBanner() launcher variant missing %q in:\n%s", want, got)
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

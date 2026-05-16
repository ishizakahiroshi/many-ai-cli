package hub

import (
	"runtime"
	"strings"
	"testing"
)

func TestStartupBannerIncludesProductDetails(t *testing.T) {
	t.Setenv("ANY_AI_CLI_WSL_LAUNCHER", "")
	t.Setenv("WSL_INTEROP", "")
	t.Setenv("WSL_DISTRO_NAME", "")
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

	if strings.Contains(got, "From Windows:") {
		t.Fatalf("startupBanner() non-WSL variant must not contain 'From Windows:' hint:\n%s", got)
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

func TestStartupBannerShowsFromWindowsHintInWSL(t *testing.T) {
	// wslutil.IsWSL is a no-op on the Windows-native build (always returns
	// false regardless of env). This test exercises Linux-side behavior and
	// can only run on non-Windows builds.
	if runtime.GOOS == "windows" {
		t.Skip("WSL detection is a Windows-native no-op; only meaningful on Linux/macOS builds")
	}
	// Pure-WSL session (no launcher) — user ran `any-ai-cli serve` directly
	// inside WSL. The Hub URL is still reachable from a Windows-side browser
	// via WSL2's automatic 127.0.0.1 forwarding, so the banner should advertise
	// the localhost form so users aren't left wondering whether they need a
	// Linux-side browser.
	t.Setenv("ANY_AI_CLI_WSL_LAUNCHER", "")
	t.Setenv("WSL_INTEROP", "/run/WSL/1_interop")
	t.Setenv("WSL_DISTRO_NAME", "Ubuntu")
	got := startupBanner("0.1.3", "127.0.0.1:47777", "abc123")

	want := "From Windows: http://localhost:47777/?token=abc123"
	if !strings.Contains(got, want) {
		t.Fatalf("startupBanner() WSL variant missing %q in:\n%s", want, got)
	}
}

func TestStartupBannerUsesAsciiUnderWindowsLauncher(t *testing.T) {
	// wslutil.IsWindowsLauncherMode / IsWSL are no-ops on the Windows-native
	// build (always false regardless of env). This test exercises Linux-side
	// behavior and can only run on non-Windows builds.
	if runtime.GOOS == "windows" {
		t.Skip("WSL launcher detection is a Windows-native no-op; only meaningful on Linux/macOS builds")
	}
	// any-ai-cli-wsl.exe sets ANY_AI_CLI_WSL_LAUNCHER=1 in the WSL shell so
	// the Linux Hub knows its stdout is being rendered by conhost.exe, where
	// EAW=Ambiguous block / box-drawing chars are promoted to full-width and
	// distort the ASCII art. The banner must fall back to plain ASCII glyphs
	// (single-byte, unambiguous width) in that mode.
	t.Setenv("ANY_AI_CLI_WSL_LAUNCHER", "1")
	t.Setenv("WSL_INTEROP", "/run/WSL/1_interop")
	t.Setenv("WSL_DISTRO_NAME", "Ubuntu")
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
		"From Windows: http://localhost:47777/?token=abc123",
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

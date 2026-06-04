package hub

import (
	"fmt"
	"strings"

	"any-ai-cli/internal/wslutil"
)

const repositoryURL = "https://github.com/ishizakahiroshi/any-ai-cli"

const (
	ansiReset        = "\x1b[0m"
	ansiBold         = "\x1b[1m"
	ansiReverse      = "\x1b[7m"
	ansiBrightOrange = "\x1b[38;5;208m"
	ansiLogoFill     = "\x1b[97m"
	ansiLogoOutline  = "\x1b[38;5;226m"
)

// unicodeLogoLines is the default banner art using block / box-drawing
// characters. Rendered cleanly on Linux, macOS, and Windows Terminal.
var unicodeLogoLines = []string{
	" █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗",
	"██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║",
	"███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║",
	"██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║",
	"██║  ██║██║ ╚████║   ██║         ██║  ██║██║",
	"╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝",
}

// asciiLogoLines is the fallback banner art used when the Hub stdout is
// rendered by the Windows conhost.exe console (i.e. via any-ai-cli-wsl.exe).
// All characters here are single-byte ASCII so the layout survives the
// East Asian Width "Ambiguous → full-width" promotion that conhost applies
// to U+2580..259F (block) and U+2500..257F (box drawing) under CJK locales.
var asciiLogoLines = []string{
	"    _    _   ___   __        _    ___        ____ _     ___ ",
	"   / \\  | \\ | \\ \\ / /       / \\  |_ _|      / ___| |   |_ _|",
	"  / _ \\ |  \\| |\\ V /  ___  / _ \\  | |  ___ | |   | |    | | ",
	" / ___ \\| |\\  | | |  |___|/ ___ \\ | | |___|| |___| |___ | | ",
	"/_/   \\_\\_| \\_| |_|       /_/   \\_\\___|     \\____|_____|___|",
}

func startupBanner(version, addr, token string) string {
	hubBase := "http://" + addr
	hubURL := hubBase + "/?token=" + token
	versionLabel := formatVersionLabel(version)
	warning := ansiBold + ansiReverse + ansiBrightOrange + " WARNING: This window is connected to the Web UI. Do not close it. " + ansiReset

	logoLines := unicodeLogoLines
	colorize := colorizeLogoLine
	if wslutil.IsWindowsLauncherMode() {
		logoLines = asciiLogoLines
		colorize = colorizeAsciiLogoLine
	}
	lines := make([]string, 0, len(logoLines)+7)
	for _, line := range logoLines {
		lines = append(lines, colorize(line))
	}
	lines = append(lines,
		"",
		fmt.Sprintf("ANY AI AGENTS                   %s", versionLabel),
		fmt.Sprintf("Runtime: %s", runtimeLabel(runtimeMode())),
		fmt.Sprintf("GitHub: %s", repositoryURL),
		fmt.Sprintf("WebUI:  %s", hubBase),
		fmt.Sprintf("Open:   %s", hubURL),
	)
	if wslutil.IsWSL() {
		// WSL2 auto-forwards 127.0.0.1 between Windows and the WSL guest, so
		// the same URL works from a Windows-side browser. Show it explicitly
		// with the "localhost" form so users running `any-ai-cli serve` inside
		// WSL know they don't need to start a Linux-side browser — the Hub UI
		// is reachable from Windows as-is.
		winURL := strings.Replace(hubURL, "127.0.0.1", "localhost", 1)
		lines = append(lines, fmt.Sprintf("From Windows: %s", winURL))
	}
	lines = append(lines,
		"",
		warning,
	)
	return strings.Join(lines, "\n") + "\n"
}

func colorizeLogoLine(line string) string {
	var b strings.Builder
	current := ""
	for _, r := range line {
		var next string
		switch r {
		case '█':
			next = ansiLogoFill
		case '╗', '╔', '╝', '╚', '║', '═':
			next = ansiLogoOutline
		default:
			next = ""
		}
		if next != current {
			if current != "" {
				b.WriteString(ansiReset)
			}
			if next != "" {
				b.WriteString(next)
			}
			current = next
		}
		b.WriteRune(r)
	}
	if current != "" {
		b.WriteString(ansiReset)
	}
	return b.String()
}

// colorizeAsciiLogoLine paints the ASCII-fallback logo. There is no
// fill / outline distinction (no `█` glyphs), so every non-space stroke
// glyph gets the same yellow outline color used for the box-drawing
// characters in the Unicode variant.
func colorizeAsciiLogoLine(line string) string {
	var b strings.Builder
	current := ""
	for _, r := range line {
		var next string
		switch r {
		case '_', '/', '\\', '|':
			next = ansiLogoOutline
		default:
			next = ""
		}
		if next != current {
			if current != "" {
				b.WriteString(ansiReset)
			}
			if next != "" {
				b.WriteString(next)
			}
			current = next
		}
		b.WriteRune(r)
	}
	if current != "" {
		b.WriteString(ansiReset)
	}
	return b.String()
}

func formatVersionLabel(version string) string {
	v := strings.TrimSpace(version)
	if v == "" {
		return "dev"
	}
	if v == "dev" || strings.HasPrefix(v, "v") {
		return v
	}
	return "v" + v
}

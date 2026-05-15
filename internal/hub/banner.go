package hub

import (
	"fmt"
	"strings"
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

func startupBanner(version, addr, token string) string {
	hubBase := "http://" + addr
	hubURL := hubBase + "/?token=" + token
	versionLabel := formatVersionLabel(version)
	warning := ansiBold + ansiReverse + ansiBrightOrange + " WARNING: This window is connected to the Web UI. Do not close it. " + ansiReset

	logoLines := []string{
		" █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗",
		"██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║",
		"███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║",
		"██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║",
		"██║  ██║██║ ╚████║   ██║         ██║  ██║██║",
		"╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝",
	}
	lines := make([]string, 0, len(logoLines)+7)
	for _, line := range logoLines {
		lines = append(lines, colorizeLogoLine(line))
	}
	lines = append(lines,
		"",
		fmt.Sprintf("Claude Code / Codex wrapper     %s", versionLabel),
		fmt.Sprintf("GitHub: %s", repositoryURL),
		fmt.Sprintf("WebUI:  %s", hubBase),
		fmt.Sprintf("Open:   %s", hubURL),
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

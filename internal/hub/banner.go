package hub

import (
	"fmt"
	"strings"
)

const repositoryURL = "https://github.com/ishizakahiroshi/any-ai-cli"

func startupBanner(version, addr, token string) string {
	hubBase := "http://" + addr
	hubURL := hubBase + "/?token=" + token
	versionLabel := formatVersionLabel(version)

	lines := []string{
		" █████╗ ███╗   ██╗██╗   ██╗       █████╗ ██╗",
		"██╔══██╗████╗  ██║╚██╗ ██╔╝      ██╔══██╗██║",
		"███████║██╔██╗ ██║ ╚████╔╝ █████╗███████║██║",
		"██╔══██║██║╚██╗██║  ╚██╔╝  ╚════╝██╔══██║██║",
		"██║  ██║██║ ╚████║   ██║         ██║  ██║██║",
		"╚═╝  ╚═╝╚═╝  ╚═══╝   ╚═╝         ╚═╝  ╚═╝╚═╝",
		"",
		fmt.Sprintf("Claude Code / Codex wrapper     %s", versionLabel),
		fmt.Sprintf("GitHub: %s", repositoryURL),
		fmt.Sprintf("WebUI:  %s", hubBase),
		fmt.Sprintf("Open:   %s", hubURL),
	}
	return strings.Join(lines, "\n") + "\n"
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

//go:build !windows && !darwin

package hub

import (
	"os/exec"

	"any-ai-cli/internal/wslutil"
)

func browserCommand(url string) *exec.Cmd {
	if wslutil.IsWindowsLauncherMode() {
		return exec.Command("explorer.exe", url)
	}
	return exec.Command("xdg-open", url)
}

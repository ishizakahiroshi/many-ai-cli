//go:build !windows && !darwin

package hub

import (
	"os"
	"os/exec"
)

func browserCommand(url string) *exec.Cmd {
	if isWSL() {
		return exec.Command("explorer.exe", url)
	}
	return exec.Command("xdg-open", url)
}

func isWSL() bool {
	return os.Getenv("WSL_INTEROP") != "" || os.Getenv("WSL_DISTRO_NAME") != ""
}

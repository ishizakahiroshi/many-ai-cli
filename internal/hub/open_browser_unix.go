//go:build !windows && !darwin

package hub

import "os/exec"

func browserCommand(url string) *exec.Cmd {
	return exec.Command("xdg-open", url)
}

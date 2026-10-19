package hub

import "os/exec"

func browserCommand(url string) *exec.Cmd {
	return exec.Command("open", url)
}

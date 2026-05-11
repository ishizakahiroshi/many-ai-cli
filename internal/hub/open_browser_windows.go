package hub

import "os/exec"

func browserCommand(url string) *exec.Cmd {
	return exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
}

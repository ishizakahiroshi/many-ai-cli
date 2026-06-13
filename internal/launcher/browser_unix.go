//go:build !windows

package launcher

import (
	"os/exec"
	"runtime"
)

// OpenBrowser launches the system default browser at url.
// On darwin, uses `open`; on linux, uses `xdg-open`.
func OpenBrowser(url string) error {
	var cmd string
	switch runtime.GOOS {
	case "darwin":
		cmd = "open"
	default:
		cmd = "xdg-open"
	}
	return exec.Command(cmd, url).Start()
}

//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"runtime"
)

func openDirNative(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", path).Start()
	default:
		if p, err := exec.LookPath("xdg-open"); err == nil {
			return exec.Command(p, path).Start()
		}
		return fmt.Errorf("no folder opener available (install xdg-utils)")
	}
}

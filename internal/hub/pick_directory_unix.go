//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"runtime"
	"strings"
)

func pickDirectoryNative() (string, error) {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("osascript", "-e", `POSIX path of (choose folder)`)
	default:
		if path, err := exec.LookPath("zenity"); err == nil {
			cmd = exec.Command(path, "--file-selection", "--directory")
		} else if path, err := exec.LookPath("kdialog"); err == nil {
			cmd = exec.Command(path, "--getexistingdirectory")
		} else {
			return "", fmt.Errorf("no folder picker available (install zenity or kdialog)")
		}
	}
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimRight(strings.TrimSpace(string(out)), "/"), nil
}

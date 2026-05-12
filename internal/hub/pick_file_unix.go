//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"strings"
)

func pickFileNative(filterExe bool) (string, error) {
	if path, err := exec.LookPath("zenity"); err == nil {
		args := []string{"--file-selection"}
		if filterExe {
			args = append(args, "--file-filter=*.AppImage;*.elf;*.sh")
		}
		cmd := exec.Command(path, args...)
		out, err := cmd.Output()
		if err != nil {
			return "", err
		}
		return strings.TrimSpace(string(out)), nil
	}
	return "", fmt.Errorf("no file picker available (install zenity)")
}

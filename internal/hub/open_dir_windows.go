//go:build windows

package hub

import "os/exec"

func openDirNative(path string) error {
	return exec.Command("explorer.exe", path).Start()
}

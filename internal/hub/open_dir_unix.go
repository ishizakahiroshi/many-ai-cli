//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"path/filepath"
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

func openRevealNative(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", "-R", path).Start()
	default:
		dir := filepath.Dir(path)
		if p, err := exec.LookPath("xdg-open"); err == nil {
			return exec.Command(p, dir).Start()
		}
		return fmt.Errorf("no folder opener available (install xdg-utils)")
	}
}

func openFileNative(filePath, app string) error {
	if app != "" {
		return exec.Command(app, filePath).Start()
	}
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", filePath).Start()
	default:
		if p, err := exec.LookPath("xdg-open"); err == nil {
			return exec.Command(p, filePath).Start()
		}
		return fmt.Errorf("no file opener available (install xdg-utils)")
	}
}

func effectiveFileOpenAppDescription(app string) string {
	if app != "" {
		return app + " <path>"
	}
	switch runtime.GOOS {
	case "darwin":
		return "open <path>"
	default:
		return "xdg-open <path>"
	}
}

func openTerminalNative(dir, app string) error {
	if app != "" {
		return exec.Command(app, dir).Start()
	}
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", "-a", "Terminal", dir).Start()
	default:
		if p, err := exec.LookPath("x-terminal-emulator"); err == nil {
			return exec.Command(p, dir).Start()
		}
		return fmt.Errorf("no terminal emulator available (install x-terminal-emulator)")
	}
}

func effectiveTerminalAppDescription(app string) string {
	if app != "" {
		return app + " <dir>"
	}
	switch runtime.GOOS {
	case "darwin":
		return "open -a Terminal <dir>"
	default:
		return "x-terminal-emulator <dir>"
	}
}

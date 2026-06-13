//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"runtime"

	"many-ai-cli/internal/wslutil"
)

// linuxOpenDefault tries the user's default opener on native Linux,
// falling back through xdg-open → gio open → gnome-open. WSL callers
// must dispatch to Windows before calling this.
func linuxOpenDefault(path string) error {
	if p, err := exec.LookPath("xdg-open"); err == nil {
		return exec.Command(p, path).Start()
	}
	if p, err := exec.LookPath("gio"); err == nil {
		return exec.Command(p, "open", path).Start()
	}
	if p, err := exec.LookPath("gnome-open"); err == nil {
		return exec.Command(p, path).Start()
	}
	return fmt.Errorf("no opener available (install xdg-utils, glib2-bin, or gnome-open)")
}

func openDirNative(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", path).Start()
	default:
		if wslutil.IsWindowsLauncherMode() {
			return exec.Command("explorer.exe", wslutil.ToWindowsPath(path)).Start()
		}
		return linuxOpenDefault(path)
	}
}

func openRevealNative(path string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", "-R", path).Start()
	default:
		if wslutil.IsWindowsLauncherMode() {
			// explorer.exe /select,<file> highlights the file inside its
			// containing folder. The comma is a separator built into the
			// /select switch and must appear in the same argument.
			return exec.Command("explorer.exe", "/select,"+wslutil.ToWindowsPath(path)).Start()
		}
		return linuxOpenDefault(filepath.Dir(path))
	}
}

func openFileNative(filePath string) error {
	switch runtime.GOOS {
	case "darwin":
		return exec.Command("open", filePath).Start()
	default:
		if wslutil.IsWindowsLauncherMode() {
			// cmd.exe /c start "" <path> dispatches to the Windows default
			// handler for files and folders alike. The empty "" is start's
			// window-title argument.
			return exec.Command("cmd.exe", "/c", "start", "", wslutil.ToWindowsPath(filePath)).Start()
		}
		return linuxOpenDefault(filePath)
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
		if wslutil.IsWindowsLauncherMode() {
			win := wslutil.ToWindowsPath(dir)
			if _, err := exec.LookPath("wt.exe"); err == nil {
				return exec.Command("wt.exe", "-d", win).Start()
			}
			return exec.Command("explorer.exe", win).Start()
		}
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
		if wslutil.IsWindowsLauncherMode() {
			return "wt.exe -d <dir>"
		}
		return "x-terminal-emulator <dir>"
	}
}

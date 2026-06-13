//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"runtime"
	"strings"

	"many-ai-cli/internal/wslutil"
)

func pickDirectoryNative() (string, error) {
	switch runtime.GOOS {
	case "darwin":
		out, err := exec.Command("osascript", "-e", `POSIX path of (choose folder)`).Output()
		if err != nil {
			return "", err
		}
		return strings.TrimRight(strings.TrimSpace(string(out)), "/"), nil
	default:
		if wslutil.IsWindowsLauncherMode() {
			return pickDirectoryViaPowerShell()
		}
		var cmd *exec.Cmd
		if path, err := exec.LookPath("zenity"); err == nil {
			cmd = exec.Command(path, "--file-selection", "--directory")
		} else if path, err := exec.LookPath("kdialog"); err == nil {
			cmd = exec.Command(path, "--getexistingdirectory")
		} else {
			return "", fmt.Errorf("no folder picker available (install zenity or kdialog)")
		}
		out, err := cmd.Output()
		if err != nil {
			return "", err
		}
		return strings.TrimRight(strings.TrimSpace(string(out)), "/"), nil
	}
}

// pickDirectoryViaPowerShell launches the Windows FolderBrowserDialog from
// inside WSL via powershell.exe (WSL interop). The returned Windows path is
// translated back to a WSL Linux path so the UI shows /mnt/<drive>/... form.
// Returns "" with no error on user cancel (no path selected).
func pickDirectoryViaPowerShell() (string, error) {
	const script = `Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dlg = New-Object System.Windows.Forms.FolderBrowserDialog
$dlg.ShowNewFolderButton = $true
if ($dlg.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $dlg.SelectedPath }`
	out, err := exec.Command("powershell.exe", "-NoProfile", "-STA", "-NonInteractive", "-Command", script).Output()
	if err != nil {
		return "", fmt.Errorf("powershell.exe folder picker failed: %w", err)
	}
	winPath := strings.TrimSpace(string(out))
	if winPath == "" {
		return "", nil
	}
	return strings.TrimRight(wslutil.ToUnixPath(winPath), "/"), nil
}

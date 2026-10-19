//go:build !windows

package hub

import (
	"fmt"
	"os/exec"
	"strings"

	"many-ai-cli/internal/wslutil"
)

func pickFileNative(filterExe bool) (string, error) {
	if wslutil.IsWindowsLauncherMode() {
		return pickFileViaPowerShell(filterExe)
	}
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

// pickFileViaPowerShell launches the Windows OpenFileDialog from inside WSL via
// powershell.exe and translates the selected Windows path back to a WSL path.
func pickFileViaPowerShell(filterExe bool) (string, error) {
	filterScript := ""
	if filterExe {
		filterScript = `$picker.Filter = "Executable (*.exe;*.bat;*.cmd;*.ps1)|*.exe;*.bat;*.cmd;*.ps1|All files (*.*)|*.*"` + "\n"
	}
	script := `
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Application]::EnableVisualStyles()
$picker = New-Object System.Windows.Forms.OpenFileDialog
` + filterScript + `
try {
  if ($picker.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $picker.FileName }
} finally {
  $picker.Dispose()
}`
	out, err := exec.Command("powershell.exe", "-NoProfile", "-STA", "-NonInteractive", "-Command", script).Output()
	if err != nil {
		return "", fmt.Errorf("powershell.exe file picker failed: %w", err)
	}
	winPath := strings.TrimSpace(string(out))
	if winPath == "" {
		return "", nil
	}
	return wslutil.ToUnixPath(winPath), nil
}

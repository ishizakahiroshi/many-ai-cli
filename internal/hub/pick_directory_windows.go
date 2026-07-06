//go:build windows

package hub

import (
	"os/exec"
	"strings"
)

func pickDirectoryNative() (string, error) {
	// C2: 共通ヘルパ wrapWithForegroundOwner (pick_ps_common.go) で
	// 隠しオーナーウィンドウ + Win32Focus 定型を差し込む。
	script := wrapWithForegroundOwner(
		`$folder = New-Object System.Windows.Forms.FolderBrowserDialog`,
		`$folder`,
		`SelectedPath`,
	)
	cmd := exec.Command("powershell.exe", "-NoProfile", "-STA", "-NonInteractive", "-Command", script)
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

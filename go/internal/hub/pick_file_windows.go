//go:build windows

package hub

import (
	"os/exec"
	"strings"
)

func pickFileNative(filterExe bool) (string, error) {
	// C2: 共通ヘルパ wrapWithForegroundOwner (pick_ps_common.go) で
	// 隠しオーナーウィンドウ + Win32Focus 定型を差し込む。
	// filterExe=true のときだけ Filter を追記する dialog 初期化スクリプトを組み立てる。
	dialogSetup := `$picker = New-Object System.Windows.Forms.OpenFileDialog`
	if filterExe {
		dialogSetup += "\n" + `$picker.Filter = "Executable (*.exe)|*.exe|All files (*.*)|*.*"`
	}
	script := wrapWithForegroundOwner(dialogSetup, `$picker`, `FileName`)
	cmd := exec.Command("powershell.exe", "-NoProfile", "-STA", "-NonInteractive", "-Command", script)
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

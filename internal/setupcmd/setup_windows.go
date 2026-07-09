//go:build windows

package setupcmd

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// createShortcuts は Windows 向け生成物を作る。
// %LOCALAPPDATA%\ManyAICLI\{start,stop}.cmd と、デスクトップの
// {Many AI Hub Start,Many AI Hub Stop}.lnk を作る。
func createShortcuts(exe string) []Result {
	var results []Result

	baseDir := filepath.Join(os.Getenv("LOCALAPPDATA"), "ManyAICLI")
	if err := os.MkdirAll(baseDir, 0o755); err != nil {
		return []Result{{Path: baseDir, Err: fmt.Errorf("mkdir: %w", err)}}
	}

	startCmd := filepath.Join(baseDir, "start.cmd")
	if err := writeWindowsCmd(startCmd, exe, "serve --open"); err != nil {
		results = append(results, Result{Path: startCmd, Err: err})
	} else {
		results = append(results, Result{Path: startCmd})
	}

	stopCmd := filepath.Join(baseDir, "stop.cmd")
	if err := writeWindowsCmd(stopCmd, exe, "stop"); err != nil {
		results = append(results, Result{Path: stopCmd, Err: err})
	} else {
		results = append(results, Result{Path: stopCmd})
	}

	desktop := resolveWindowsDesktop()
	if desktop == "" {
		results = append(results, Result{Path: "desktop shortcut", Err: fmt.Errorf("desktop directory not found")})
		return results
	}

	startLnk := filepath.Join(desktop, "Many AI Hub Start.lnk")
	if err := createWindowsShortcut(startLnk, startCmd, exe); err != nil {
		results = append(results, Result{Path: startLnk, Err: err})
	} else {
		results = append(results, Result{Path: startLnk})
	}

	stopLnk := filepath.Join(desktop, "Many AI Hub Stop.lnk")
	if err := createWindowsShortcut(stopLnk, stopCmd, exe); err != nil {
		results = append(results, Result{Path: stopLnk, Err: err})
	} else {
		results = append(results, Result{Path: stopLnk})
	}

	return results
}

// writeWindowsCmd は cd /d %USERPROFILE% → call "<exe>" <args> → pause の 4 行 bat を書き出す。
// pnpm/npm shim (.CMD) 経由でも呼べるように call を必ず付ける（実機検証済み形式）。
func writeWindowsCmd(path, exe, args string) error {
	body := "@echo off\r\n" +
		"cd /d %USERPROFILE%\r\n" +
		"call \"" + exe + "\" " + args + "\r\n" +
		"pause\r\n"
	return os.WriteFile(path, []byte(body), 0o644)
}

// resolveWindowsDesktop はデスクトップディレクトリを解決する。
// OneDrive リダイレクト環境も想定して PowerShell の GetFolderPath を優先し、
// 失敗時は %USERPROFILE%\Desktop へフォールバックする。
func resolveWindowsDesktop() string {
	out, err := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command",
		"[Environment]::GetFolderPath('Desktop')").Output()
	if err == nil {
		p := strings.TrimSpace(string(out))
		if p != "" {
			return p
		}
	}
	if home, err := os.UserHomeDir(); err == nil {
		return filepath.Join(home, "Desktop")
	}
	return ""
}

// createWindowsShortcut は PowerShell の WScript.Shell 経由で .lnk を作る。
// 新規依存を入れず（COM を Go から直接叩かず）、Windows 標準の PowerShell に任せる。
func createWindowsShortcut(lnkPath, targetCmd, iconExe string) error {
	// シングルクォート内に埋め込むためエスケープ（' → '' が PowerShell 流儀）。
	esc := func(s string) string { return strings.ReplaceAll(s, "'", "''") }
	script := fmt.Sprintf(
		`$s=(New-Object -ComObject WScript.Shell).CreateShortcut('%s');$s.TargetPath='%s';$s.WorkingDirectory='%s';$s.IconLocation='%s,0';$s.Save()`,
		esc(lnkPath), esc(targetCmd), esc(filepath.Dir(targetCmd)), esc(iconExe),
	)
	return exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", script).Run()
}

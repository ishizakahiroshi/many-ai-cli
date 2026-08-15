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
	if err := os.MkdirAll(baseDir, 0o755); err != nil { // #nosec G703 -- LOCALAPPDATA is the intended per-user Windows data root.
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

	// デスクトップのアイコンは 1 個（トレイ常駐）に寄せる。起動も停止もトレイの
	// メニューから行えるので、Start と Stop を分ける理由が無くなった。
	// .cmd を挟まず exe を直接指すのは、.cmd だとコンソール窓が残るため。
	// WindowStyle=7（最小化）と tray 側の FreeConsole の 2 段で窓を出さない。
	trayLnk := filepath.Join(desktop, "Many AI Hub.lnk")
	if err := createWindowsShortcutWithArgs(trayLnk, exe, exe, "tray", 7); err != nil {
		results = append(results, Result{Path: trayLnk, Err: err})
	} else {
		results = append(results, Result{Path: trayLnk})
	}

	// 既存利用者のデスクトップにある 2 個は消さない。勝手に消すと使い慣れた導線が
	// 黙って変わる。残っていることだけ知らせて、消すかどうかは利用者が決める。
	// start.cmd / stop.cmd も残す（旧ショートカットの参照先なので、消すと
	// 「アイコンはあるが動かない」状態になる）。
	for _, name := range []string{"Many AI Hub Start.lnk", "Many AI Hub Stop.lnk"} {
		old := filepath.Join(desktop, name)
		if _, statErr := os.Stat(old); statErr == nil {
			results = append(results, Result{
				Path: old,
				Note: "旧アイコンです。「Many AI Hub」1 個で起動も停止もできます。不要なら手動で削除してください（自動では消しません）",
			})
		}
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
	return os.WriteFile(path, []byte(body), 0o644) // #nosec G703 -- path is one of the fixed files under LOCALAPPDATA.
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

// createWindowsShortcutWithArgs は PowerShell の WScript.Shell 経由で .lnk を作る。
// 新規依存を入れず（COM を Go から直接叩かず）、Windows 標準の PowerShell に任せる。
// windowStyle は WScript.Shell の規約で 1=通常 / 7=最小化。
func createWindowsShortcutWithArgs(lnkPath, target, iconExe, args string, windowStyle int) error {
	// シングルクォート内に埋め込むためエスケープ（' → '' が PowerShell 流儀）。
	esc := func(s string) string { return strings.ReplaceAll(s, "'", "''") }
	script := fmt.Sprintf(
		`$s=(New-Object -ComObject WScript.Shell).CreateShortcut('%s');$s.TargetPath='%s';$s.Arguments='%s';$s.WorkingDirectory='%s';$s.WindowStyle=%d;$s.IconLocation='%s,0';$s.Save()`,
		esc(lnkPath), esc(target), esc(args), esc(filepath.Dir(target)), windowStyle, esc(iconExe),
	)
	return exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", script).Run() // #nosec G702 -- script is a fixed COM shortcut template with path values single-quote escaped.
}

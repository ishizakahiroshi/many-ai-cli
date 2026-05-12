//go:build windows

package hub

import "os/exec"

func openDirNative(path string) error {
	return exec.Command("explorer.exe", path).Start()
}

func openRevealNative(path string) error {
	return exec.Command("explorer.exe", "/select,"+path).Start()
}

func openFileNative(filePath, app string) error {
	if app != "" {
		return exec.Command(app, filePath).Start()
	}
	return exec.Command("cmd", "/c", "start", "", filePath).Start()
}

func effectiveFileOpenAppDescription(app string) string {
	if app != "" {
		return app + " <path>"
	}
	return `cmd /c start "" <path>`
}

func openTerminalNative(dir, app string) error {
	if app != "" {
		return exec.Command(app, dir).Start()
	}
	if err := exec.Command("wt.exe", "-d", dir).Start(); err == nil {
		return nil
	}
	return exec.Command("powershell.exe", "-NoExit", "-Command", "Set-Location -LiteralPath $args[0]", dir).Start()
}

func effectiveTerminalAppDescription(app string) string {
	if app != "" {
		return app + " <dir>"
	}
	return `wt.exe -d <dir> (fallback: powershell.exe -NoExit -Command "Set-Location -LiteralPath $args[0]" <dir>)`
}

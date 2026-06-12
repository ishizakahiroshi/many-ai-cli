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
	// app 未指定は「OS 既定の関連付けアプリで開く」。explorer.exe <path>（/select なし）は
	// 既定ハンドラへディスパッチするため .html ならブラウザ等で開く。shell を介さないので
	// cmd.exe メタ文字リスクもない。フォルダ内で選択表示したい場合は openRevealNative を使う。
	return exec.Command("explorer.exe", filePath).Start()
}

func effectiveFileOpenAppDescription(app string) string {
	if app != "" {
		return app + " <path>"
	}
	return `explorer.exe <path>`
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

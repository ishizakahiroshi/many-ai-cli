//go:build windows

package hub

import (
	"os/exec"

	"golang.org/x/sys/windows"
)

func openDirNative(path string) error {
	return exec.Command("explorer.exe", path).Start()
}

func openRevealNative(path string) error {
	return exec.Command("explorer.exe", "/select,"+path).Start()
}

func openFileNative(filePath string) error {
	// 「OS 既定の関連付けアプリで開く」。
	// explorer.exe <path> は画像など一部のファイル種別で既定アプリを起動せず
	// 親フォルダを開いてしまう（"画像を開く" を押すとフォルダが開く不具合の原因）。
	// ShellExecute の "open" 動詞なら既定ハンドラへ確実にディスパッチでき、
	// shell を介さないので cmd.exe メタ文字リスクもない。
	// フォルダ内で選択表示したい場合は openRevealNative を使う。
	return shellExecuteOpen(filePath)
}

// shellExecuteOpen は ShellExecute("open") で path を OS 既定の関連付けで開く。
// ファイル・フォルダ・URL いずれにも使え、戻りは即時（プロセス終了を待たない）。
func shellExecuteOpen(path string) error {
	verb, err := windows.UTF16PtrFromString("open")
	if err != nil {
		return err
	}
	file, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	return windows.ShellExecute(0, verb, file, nil, nil, windows.SW_SHOWNORMAL)
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

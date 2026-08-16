//go:build windows

package uninstall

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// removeAutostart は setup がスタートアップフォルダに置いたトレイのショートカットを消す。
// 残したままバイナリを消すと、ログインのたびに存在しない exe を起動しようとする
// 置き去りになる（利用者のファイルへ書く機能は回収経路まで持つ、の一環）。
// 場所の解決は setupcmd.resolveWindowsStartup と揃える（GetFolderPath 優先・%APPDATA% 予備）。
func removeAutostart() {
	dir := ""
	out, err := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command",
		"[Environment]::GetFolderPath('Startup')").Output()
	if err == nil {
		dir = strings.TrimSpace(string(out))
	}
	if dir == "" {
		if appData := os.Getenv("APPDATA"); appData != "" {
			dir = filepath.Join(appData, "Microsoft", "Windows", "Start Menu", "Programs", "Startup")
		}
	}
	if dir == "" {
		return
	}

	lnk := filepath.Join(dir, "Many AI Hub.lnk")
	if err := os.Remove(lnk); err == nil { // #nosec G703 -- fixed file name under the current user's own Startup folder, which is exactly what setup wrote.
		fmt.Printf("削除しました: %s\n", lnk)
	}
}

// removeSelf は PowerShell の遅延削除で実行中バイナリ自体を消す。
// Windows ではプロセスが保持するファイルは即時削除できないため、
// 親プロセス終了後に別プロセスで Remove-Item する。
func removeSelf() error {
	exe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("バイナリパスの取得に失敗: %w", err)
	}

	// パスを PowerShell 単一引用リテラルとして安全に渡す（" ` $() 等を含むパス対策）。
	// -LiteralPath はワイルドカード解釈をしない。単一引用内の ' は '' に二重化する。
	escaped := strings.ReplaceAll(exe, "'", "''")
	script := fmt.Sprintf(`Start-Sleep -Seconds 2; Remove-Item -LiteralPath '%s' -Force`, escaped)
	cmd := exec.Command("powershell", "-WindowStyle", "Hidden", "-NonInteractive", "-Command", script)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("バイナリの削除に失敗: %w\n手動で削除してください: %s", err, exe)
	}

	fmt.Printf("バイナリを削除中: %s\n", exe)
	fmt.Println("\nアンインストール完了。")
	return nil
}

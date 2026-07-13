//go:build windows

package uninstall

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
)

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

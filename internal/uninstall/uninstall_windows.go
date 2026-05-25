//go:build windows

package uninstall

import (
	"fmt"
	"os"
	"os/exec"
)

// removeSelf は PowerShell の遅延削除で実行中バイナリ自体を消す。
// Windows ではプロセスが保持するファイルは即時削除できないため、
// 親プロセス終了後に別プロセスで Remove-Item する。
func removeSelf() error {
	exe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("バイナリパスの取得に失敗: %w", err)
	}

	script := fmt.Sprintf(`Start-Sleep -Seconds 2; Remove-Item -Force "%s"`, exe)
	cmd := exec.Command("powershell", "-WindowStyle", "Hidden", "-NonInteractive", "-Command", script)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("バイナリの削除に失敗: %w\n手動で削除してください: %s", err, exe)
	}

	fmt.Printf("バイナリを削除中: %s\n", exe)
	fmt.Println("\nアンインストール完了。")
	return nil
}

//go:build !windows

package uninstall

import (
	"fmt"
	"os"
)

func removeSelf() error {
	exe, err := os.Executable()
	if err != nil {
		return fmt.Errorf("バイナリパスの取得に失敗: %w", err)
	}

	if err := os.Remove(exe); err != nil {
		return fmt.Errorf("バイナリの削除に失敗: %w\n手動で削除してください: %s", err, exe)
	}

	fmt.Printf("バイナリを削除しました: %s\n", exe)
	fmt.Println("\nアンインストール完了。")
	return nil
}

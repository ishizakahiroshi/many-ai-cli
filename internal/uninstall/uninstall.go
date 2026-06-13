package uninstall

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Run はアンインストールの主処理。
// purge=true のときバイナリ本体も削除する（OS別実装）。
func Run(purge bool) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("ホームディレクトリの取得に失敗: %w", err)
	}
	dataDir := filepath.Join(home, ".many-ai-cli")

	if !confirm(dataDir, purge) {
		fmt.Println("アンインストールをキャンセルしました。")
		return nil
	}

	if _, err := os.Stat(dataDir); err == nil {
		if err := os.RemoveAll(dataDir); err != nil {
			return fmt.Errorf("データディレクトリの削除に失敗: %w", err)
		}
		fmt.Printf("削除しました: %s\n", dataDir)
	} else {
		fmt.Printf("存在しないためスキップ: %s\n", dataDir)
	}

	printBrowserNote()

	if purge {
		return removeSelf()
	}

	exe, err := os.Executable()
	if err == nil {
		fmt.Printf("\nバイナリを手動で削除してください:\n  %s\n", exe)
	}
	fmt.Println("\nアンインストール完了。")
	return nil
}

// printBrowserNote は CLI からは消せないブラウザ側 localStorage の残留を案内する。
// テーマ・言語・お気に入り等の UI 設定のみで機密値は含まないが、
// 「完全アンインストール」のために手動消去の手順を示す。
func printBrowserNote() {
	fmt.Println("\nブラウザに保存された UI 設定（テーマ・言語・お気に入り・レイアウト等）は残ります。")
	fmt.Println("消去するには Hub を開いていたタブで DevTools コンソール（F12）を開き、次を実行してください:")
	fmt.Println("  localStorage.clear()")
}

func confirm(dataDir string, purge bool) bool {
	fmt.Println("以下を削除します:")
	fmt.Printf("  設定・ログ: %s\n", dataDir)
	if purge {
		if exe, err := os.Executable(); err == nil {
			fmt.Printf("  バイナリ:   %s\n", exe)
		}
	}
	fmt.Print("\n続行しますか? [y/N]: ")
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Scan()
	ans := strings.TrimSpace(strings.ToLower(scanner.Text()))
	return ans == "y" || ans == "yes"
}

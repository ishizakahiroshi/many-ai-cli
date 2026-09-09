// Package setupcmd は `many-ai-cli setup` の実装。
// インストール後にローカル生成のダブルクリック起動物（Windows: .cmd + .lnk /
// macOS: .command / Linux: .desktop）を作り、一般ユーザーがターミナルを
// 経由せずに Hub を起動できるようにする。
package setupcmd

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"many-ai-cli/internal/config"
)

// Result は生成物 1 件の結果。Err が nil なら成功。
// Note が入っているものは「作った物」ではなく「見つけた物への案内」で、
// [OK] created ではなく [NOTE] として出す（旧アイコンの残存など）。
type Result struct {
	Path string
	Err  error
	Note string
}

// Run は `many-ai-cli setup` の本体。
func Run() error {
	exe, err := resolveExecutable()
	if err != nil {
		return fmt.Errorf("resolve executable: %w", err)
	}

	fmt.Println("Many AI CLI setup")
	fmt.Println()
	fmt.Printf("[OK] executable: %s\n", exe)

	cfgDir, err := config.Dir()
	if err != nil {
		return fmt.Errorf("resolve config dir: %w", err)
	}
	if err := os.MkdirAll(cfgDir, 0o755); err != nil {
		return fmt.Errorf("create config dir: %w", err)
	}
	fmt.Printf("[OK] config directory: %s\n", cfgDir)

	results := createShortcuts(exe)
	hasFail := false
	for _, r := range results {
		if r.Err != nil {
			fmt.Printf("[FAIL] %s: %v\n", r.Path, r.Err)
			hasFail = true
			continue
		}
		if r.Note != "" {
			fmt.Printf("[NOTE] %s: %s\n", r.Path, r.Note)
			continue
		}
		fmt.Printf("[OK] created: %s\n", r.Path)
	}

	fmt.Println()
	fmt.Println("Next:")
	if runtime.GOOS == "windows" {
		// Windows は v0.7.0 からトレイ常駐 1 個に寄せてある（Start / Stop の 2 個ではない）。
		fmt.Println(`  Double click "MANY-AI-CLI" on your desktop. It also starts on sign-in.`)
		fmt.Println(`  To stop that, delete "MANY-AI-CLI" from the Startup folder`)
		fmt.Println(`  (or turn it off in Task Manager > Startup apps).`)
	} else {
		fmt.Println(`  Double click "Many AI Hub Start" on your desktop.`)
	}
	if runtime.GOOS == "linux" {
		fmt.Println(`  (GNOME: right click the desktop icon and choose "Allow Launching" the first time.)`)
	}

	if hasFail {
		return errors.New("some shortcuts failed to create")
	}
	return nil
}

// resolveExecutable は自プロセスの実体絶対パス（symlink 解決済み）を返す。
func resolveExecutable() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	abs, err := filepath.Abs(exe)
	if err != nil {
		return exe, nil
	}
	return abs, nil
}

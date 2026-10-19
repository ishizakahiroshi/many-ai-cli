//go:build !windows

package securefile

import (
	"fmt"
	"os"
)

// RestrictFile は非 Windows では既存の 0o600 chmod で十分と判断し追加処理を行わない。
// 呼び出し元は本呼び出し前に `os.Chmod(path, 0o600)` を実施している想定 (POSIX の
// パーミッションビットは OS レベルで機能するため)。
func RestrictFile(path string) error {
	if _, err := os.Stat(path); err != nil {
		return fmt.Errorf("securefile.RestrictFile stat: %w", err)
	}
	return nil
}

// EnsurePrivateDir は非 Windows では `os.MkdirAll` + `os.Chmod(dir, 0o700)` に倒す。
// 既存挙動と変わらないが、呼び出し元から見た API を Windows と揃えるための薄いラッパー。
func EnsurePrivateDir(path string) error {
	if err := os.MkdirAll(path, 0o700); err != nil {
		return fmt.Errorf("securefile.EnsurePrivateDir mkdir: %w", err)
	}
	if err := os.Chmod(path, 0o700); err != nil {
		return fmt.Errorf("securefile.EnsurePrivateDir chmod: %w", err)
	}
	return nil
}

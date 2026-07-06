//go:build !linux && !windows
// +build !linux,!windows

package hub

import (
	"errors"
	"os"
)

// atomicRenameNoReplace は macOS/BSD 等のフォールバック実装。
// `renameatx_np(RENAME_EXCL)` (macOS 10.12+) や `renameat2` (Linux) と等価の
// atomic API は golang.org/x/sys が現時点で薄いラッパーを提供していないため、
// 事前 Lstat + os.Rename の 2 ステップで近似する（TOCTOU が理論上残る）。
// これは README で「native macOS は fully verified ではない」と明記済みの
// プラットフォームであり、Linux/Windows の主要動作環境より弱い保証で妥協する。
func atomicRenameNoReplace(src, dst string) error {
	if _, err := os.Lstat(dst); err == nil {
		return os.ErrExist
	} else if !os.IsNotExist(err) {
		return err
	}
	return os.Rename(src, dst)
}

// isRenameTargetExistsErr は atomicRenameNoReplace が「dst 既存」を理由に失敗
// したことを返す。呼び出し側の 409 Conflict 応答用。
func isRenameTargetExistsErr(err error) bool {
	if err == nil {
		return false
	}
	return errors.Is(err, os.ErrExist)
}

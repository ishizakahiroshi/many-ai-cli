//go:build linux
// +build linux

package hub

import (
	"errors"
	"os"

	"golang.org/x/sys/unix"
)

// atomicRenameNoReplace は Linux で renameat2 の RENAME_NOREPLACE フラグ付き呼び出しに
// 委譲する (kernel 3.15+ で追加)。既存だと EEXIST を返す。カーネルが古いか
// ファイルシステムが未対応の場合は EINVAL を返すので、その時のみ os.Rename
// フォールバック + 事後 stat 確認で近似する（原子性は失われるが実運用では発生稀）。
func atomicRenameNoReplace(src, dst string) error {
	err := unix.Renameat2(unix.AT_FDCWD, src, unix.AT_FDCWD, dst, unix.RENAME_NOREPLACE)
	if err == nil {
		return nil
	}
	if errors.Is(err, unix.EINVAL) || errors.Is(err, unix.ENOSYS) {
		// フォールバック: 事後 stat で dst 実体が変わらなかったかざっくり確認する
		// (完全な TOCTOU 対策ではないが、renameat2 不対応環境向けの最善努力)。
		if _, statErr := os.Lstat(dst); statErr == nil {
			return os.ErrExist
		}
		return os.Rename(src, dst)
	}
	return err
}

// isRenameTargetExistsErr は atomicRenameNoReplace が「dst 既存」を理由に失敗
// したことを返す。呼び出し側の 409 Conflict 応答用。
func isRenameTargetExistsErr(err error) bool {
	if err == nil {
		return false
	}
	return errors.Is(err, unix.EEXIST) || errors.Is(err, os.ErrExist)
}

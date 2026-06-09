//go:build !windows

package hub

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/unix"
)

// ensureDownloadRoom は dir のあるファイルシステムの空き容量を Statfs で確認し、
// requiredBytes に満たなければエラーを返す（Windows の GetDiskFreeSpaceEx 相当）。
func ensureDownloadRoom(dir string, requiredBytes int64) error {
	if requiredBytes <= 0 {
		return nil
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	abs, err := filepath.Abs(dir)
	if err != nil {
		return err
	}
	var st unix.Statfs_t
	if err := unix.Statfs(abs, &st); err != nil {
		return err
	}
	// Bavail = 非特権ユーザーが使えるブロック数、Bsize = ブロックサイズ。
	// 型は OS で異なる（linux: Bsize int64 / darwin: uint32）ため uint64 へ揃える。
	avail := uint64(st.Bavail) * uint64(st.Bsize)
	if avail < uint64(requiredBytes) {
		return fmt.Errorf("not enough free disk space for Whisper download: need at least %.1f MiB, available %.1f MiB", float64(requiredBytes)/(1024*1024), float64(avail)/(1024*1024))
	}
	return nil
}

//go:build windows

package hub

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/windows"
)

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
	ptr, err := windows.UTF16PtrFromString(abs)
	if err != nil {
		return err
	}
	var freeBytes uint64
	if err := windows.GetDiskFreeSpaceEx(ptr, &freeBytes, nil, nil); err != nil {
		return err
	}
	if freeBytes < uint64(requiredBytes) {
		return fmt.Errorf("not enough free disk space for Whisper download: need at least %.1f MiB, available %.1f MiB", float64(requiredBytes)/(1024*1024), float64(freeBytes)/(1024*1024))
	}
	return nil
}

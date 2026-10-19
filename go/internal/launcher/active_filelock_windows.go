//go:build windows

package launcher

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/windows"
)

// withActiveFileCrossProcess serializes load→modify→save of launcher-active.json
// across processes via LockFileEx on a sibling .lock file.
func withActiveFileCrossProcess(fn func() error) error {
	path, err := activePath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("mkdir active dir: %w", err)
	}
	lockPath := path + ".lock"
	f, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return fmt.Errorf("open active file lock: %w", err)
	}
	defer f.Close()

	// Lock the first byte exclusively (blocking).
	var ol windows.Overlapped
	err = windows.LockFileEx(
		windows.Handle(f.Fd()),
		windows.LOCKFILE_EXCLUSIVE_LOCK,
		0,
		1,
		0,
		&ol,
	)
	if err != nil {
		return fmt.Errorf("LockFileEx active file: %w", err)
	}
	defer func() {
		var uol windows.Overlapped
		_ = windows.UnlockFileEx(windows.Handle(f.Fd()), 0, 1, 0, &uol)
	}()
	return fn()
}

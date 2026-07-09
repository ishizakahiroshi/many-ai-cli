//go:build unix

package launcher

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/unix"
)

// withActiveFileCrossProcess serializes load→modify→save of launcher-active.json
// across processes via flock on a sibling .lock file.
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
	if err := unix.Flock(int(f.Fd()), unix.LOCK_EX); err != nil {
		return fmt.Errorf("flock active file: %w", err)
	}
	defer func() { _ = unix.Flock(int(f.Fd()), unix.LOCK_UN) }()
	return fn()
}

//go:build windows

package wrapper

import "os"

// processAlive reports whether pid refers to a running process.
// On Windows, os.FindProcess opens a real process handle and fails when the
// PID does not exist.
func processAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	p, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	_ = p.Release()
	return true
}

//go:build windows

package launcher

import "os"

// pidAlive reports whether pid refers to a running process. On Windows,
// os.FindProcess opens a real process handle and fails when the PID does
// not exist. This is only the first guard — callers must also probe the
// recorded Hub URL because a dead launcher's PID can be reused by an
// unrelated process (same rationale as internal/hub/lifecycle.go
// killStalePid).
func pidAlive(pid int) bool {
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

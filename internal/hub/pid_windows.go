//go:build windows

package hub

import "os"

// pidAlive reports whether pid refers to a running process. On Windows,
// os.FindProcess opens a real process handle and fails when the PID does
// not exist. This is only the first guard — callers must also probe the
// recorded Hub port because a dead Hub's PID can be reused by an
// unrelated process (same rationale as killStalePid and
// internal/launcher/pid_windows.go).
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

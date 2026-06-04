//go:build !windows

package hub

import (
	"os"
	"syscall"
)

// pidAlive reports whether pid refers to a running process. On Unix,
// os.FindProcess always succeeds, so signal 0 is used to test existence.
// This is only the first guard — callers must also probe the recorded
// Hub port (see runfile.go).
func pidAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	p, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	return p.Signal(syscall.Signal(0)) == nil
}

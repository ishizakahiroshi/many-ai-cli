//go:build unix

package wrapper

import (
	"os"
	"syscall"
)

// processAlive reports whether pid refers to a running process.
// On Unix, os.FindProcess always succeeds; Signal(0) is the liveness probe.
func processAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	p, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	err = p.Signal(syscall.Signal(0))
	return err == nil
}

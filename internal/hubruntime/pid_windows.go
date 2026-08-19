//go:build windows

package hubruntime

import "syscall"

// processQueryLimitedInformation is PROCESS_QUERY_LIMITED_INFORMATION; the
// syscall package does not export it.
const processQueryLimitedInformation = 0x1000

// stillActive is STILL_ACTIVE, what GetExitCodeProcess returns while a process
// is running. A process that exits with code 259 is therefore reported as
// alive; that is a known and accepted limitation of this API. It does not
// weaken the guard, because the caller's endpoint probe is what actually
// decides, and neither does it detect PID reuse (see PIDAlive).
const stillActive = 259

// pidAlive is the Windows implementation of PIDAlive.
//
// os.FindProcess is not usable here: on Windows it opens a handle that is
// still obtainable for an already-exited process, so it answers "alive" for
// anything whose handle can be opened. GetExitCodeProcess is the real check.
func pidAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	h, err := syscall.OpenProcess(processQueryLimitedInformation, false, uint32(pid))
	if err != nil {
		return false
	}
	defer func() { _ = syscall.CloseHandle(h) }()
	var code uint32
	if err := syscall.GetExitCodeProcess(h, &code); err != nil {
		return false
	}
	return code == stillActive
}

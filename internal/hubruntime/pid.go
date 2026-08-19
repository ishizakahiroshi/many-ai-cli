package hubruntime

// PIDAlive reports whether pid refers to a running process.
//
// This is the single implementation for the whole repository: Hub, wrapper and
// launcher all delegate here. Four hand-written copies existed before and were
// free to drift apart while every one of them guarded the same thing — whether
// a recorded PID in a runtime ledger still belongs to a live process.
//
// It is only the FIRST guard. A dead process's PID is reused by the OS, so a
// "true" answer does not prove the recorded process is the one still running.
// Callers must pair this with an identity check on the recorded endpoint (see
// RunningPortWith, which also probes /api/info with this machine's token).
func PIDAlive(pid int) bool {
	return pidAlive(pid)
}

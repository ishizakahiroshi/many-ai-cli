package hub

import "many-ai-cli/internal/hubruntime"

// pidAlive reports whether pid refers to a running process.
//
// The OS-specific implementation lives in internal/hubruntime so Hub, wrapper
// and launcher cannot drift apart: all three guard the same thing (a PID
// recorded in a runtime ledger) and previously carried four separate copies of
// it. See hubruntime.PIDAlive for why this is only the first guard.
func pidAlive(pid int) bool {
	return hubruntime.PIDAlive(pid)
}

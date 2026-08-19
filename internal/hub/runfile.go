// Hub runtime file compatibility wrappers.
//
// The implementation lives in internal/hubruntime because both Hub and
// wrapper need the same fallback-port and stale-ledger semantics. Keep these
// names local to hub so existing lifecycle tests and callers remain stable.
package hub

import (
	"many-ai-cli/internal/hubruntime"
)

const hubRuntimeFile = hubruntime.FileName

type hubRuntimeData = hubruntime.Data

func hubRuntimePath() (string, error) { return hubruntime.Path() }

func writeHubRuntime(port int) error { return hubruntime.Write(port) }

func readHubRuntime() (*hubRuntimeData, error) { return hubruntime.Read() }

func removeHubRuntimeIfPID(pid int) { hubruntime.RemoveIfPID(pid) }

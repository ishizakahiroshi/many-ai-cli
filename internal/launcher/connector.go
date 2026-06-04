package launcher

import "context"

// Connector abstracts the underlying transport (WSL / SSH) used to reach a Hub.
// Callers pass a Profile and receive a channel that emits the Hub URL once the
// Hub is ready, plus an error channel for fatal failures.
//
// Start must be non-blocking: it launches the connection in a goroutine and
// returns immediately. The caller is responsible for cancelling ctx to trigger
// shutdown. Start returns an error synchronously only if the Profile is
// obviously misconfigured before any I/O begins.
type Connector interface {
	// Start initiates the connection described by p. On success it eventually
	// sends one Hub URL string on urlCh and then closes it. On failure it
	// sends one non-nil error on errCh.
	//
	// errCh is closed only when the connection has terminated — after a
	// failure (the error is sent first), after the underlying child process
	// exits (e.g. the remote Hub was stopped from the Web UI), or after ctx
	// cancellation. Callers must treat the close as "connection over" and
	// release whatever depends on it (the CLI launcher exits its process so
	// the console window closes).
	//
	// Cancelling ctx causes the connector to shut down and perform its
	// cleanup (e.g. kill remote processes). After ctx is cancelled both
	// channels will be drained and closed before the internal goroutine exits.
	Start(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) error
}

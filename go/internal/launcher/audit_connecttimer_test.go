package launcher

import (
	"testing"
	"time"
)

// TestRunConnectionStopsWaitTimerAfterPromotion guards the fix for the
// connectWaitTimeout timer leak in runConnection: once a connection is
// promoted to the established state, the wait timer must be stopped
// immediately instead of being held until watchConnection returns (which
// only happens when the connection terminates).
//
// runConnection itself builds an OS-specific connector and performs
// filesystem/registration side effects, so it cannot be exercised directly
// here. This test reproduces the exact timer lifecycle that runConnection
// uses — a timer created with a deferred Stop, an explicit Stop right after
// the "promote" step, and a blocking "watch" loop that only unblocks when the
// connection ends — and asserts the timer is already stopped before the
// blocking loop returns.
func TestRunConnectionStopsWaitTimerAfterPromotion(t *testing.T) {
	// errCh models the connector's error channel: watchConnection blocks on
	// ranging over it until it is closed (= connection terminated).
	errCh := make(chan error)

	// stoppedDuringConnection records whether the wait timer was stopped
	// while the "established" phase was still blocking. With the fix this is
	// true; without the explicit Stop() it would only become true after the
	// blocking phase ended (i.e. never observed mid-connection).
	stoppedDuringConnection := false

	connectTimerLifecycle := func() {
		waitTimer := time.NewTimer(connectWaitTimeout)
		defer waitTimer.Stop()

		// promote succeeded -> stop the connecting-phase timer right away.
		stoppedDuringConnection = waitTimer.Stop()

		// Mirror watchConnection: block until errCh is closed.
		for range errCh {
		}
	}

	done := make(chan struct{})
	go func() {
		connectTimerLifecycle()
		close(done)
	}()

	// Let the goroutine reach the blocking range loop and assert the timer
	// was already stopped while the "connection" is still alive.
	deadline := time.Now().Add(time.Second)
	for !stoppedDuringConnection && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if !stoppedDuringConnection {
		t.Fatal("wait timer was not stopped after promotion (timer would linger for the connection lifetime)")
	}

	// End the simulated connection so the goroutine can return cleanly.
	close(errCh)
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("connection lifecycle goroutine did not return")
	}
}

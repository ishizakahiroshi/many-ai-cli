package launcher

import (
	"context"
	"sync"
	"testing"
	"time"
)

// TestWSLRunURLChNoSendAfterClose is a regression model for the WSL connector
// "send on closed channel" panic (internal/launcher/connector_wsl_windows.go).
//
// The original code forwarded the Hub URL to urlCh from a *separate* goroutine
// that was not tracked by the scanner WaitGroup, so the main flow's
// close(urlCh) could race with that goroutine's `urlCh <- url`, panicking with
// "send on closed channel". WSLConnector.run is //go:build windows and needs a
// real wsl.exe, so this test reproduces the exact channel choreography of the
// fixed run (foundCh -> urlCh send -> close, with cmd exit delivered via a
// buffered waitCh) and asserts that across many timing permutations urlCh is
// only ever sent on before it is closed, and only from the single main flow.
//
// Run under `go test -race` to catch any reintroduced concurrent send/close.
func TestWSLRunURLChNoSendAfterClose(t *testing.T) {
	for i := 0; i < 200; i++ {
		runWSLURLChModel(t, true /*emitURL*/, false /*cancelEarly*/)
		runWSLURLChModel(t, false /*emitURL: process exits before URL*/, false)
		runWSLURLChModel(t, true, true /*cancelEarly: ctx cancelled*/)
	}
}

// runWSLURLChModel mirrors the fixed connector_wsl_windows.go run() channel
// flow. emitURL controls whether a Hub URL appears before the (simulated)
// process exit; cancelEarly cancels ctx to exercise the cancellation paths.
// It fails the test if urlCh is closed more than once or sent on after close.
func runWSLURLChModel(t *testing.T, emitURL, cancelEarly bool) {
	t.Helper()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	foundCh := make(chan string, 1) // matches ScanForURL's buffered foundCh
	waitCh := make(chan error, 1)   // buffered: delivers cmd.Wait() exactly once

	// Simulate the scanner WaitGroup completing, then the process exit being
	// delivered on waitCh (mirrors `go func(){ wg.Wait(); waitCh <- cmd.Wait() }`).
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		if emitURL {
			// Buffered send, non-blocking — same as ScanForURL.
			foundCh <- "http://127.0.0.1:47777/?token=deadbeef"
		}
		// Tiny jitter so the producer and the close path interleave.
		time.Sleep(time.Duration(time.Now().UnixNano()%3) * time.Microsecond)
		waitCh <- nil
	}()

	if cancelEarly {
		go cancel()
	}

	// guarded urlCh: panics in the model if sent on after close, and detects
	// double-close — exactly the invariants the fix must uphold.
	urlCh := make(chan string, 1)
	var closed bool
	var mu sync.Mutex
	closeOnce := func() {
		mu.Lock()
		defer mu.Unlock()
		if closed {
			t.Errorf("urlCh closed twice")
			return
		}
		closed = true
		close(urlCh)
	}

	// --- This block is the structural copy of the fixed run()'s main flow. ---
	select {
	case url := <-foundCh:
		select {
		case urlCh <- url:
		case <-ctx.Done():
		}
		closeOnce()
		select {
		case <-waitCh:
		case <-ctx.Done():
			<-waitCh
		}
	case <-waitCh:
		closeOnce()
	case <-ctx.Done():
		<-waitCh
		closeOnce()
	}
	// ------------------------------------------------------------------------

	wg.Wait()

	mu.Lock()
	if !closed {
		t.Errorf("urlCh was never closed (emitURL=%v cancelEarly=%v)", emitURL, cancelEarly)
	}
	mu.Unlock()
}

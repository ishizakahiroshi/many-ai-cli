package hub

import (
	"io"
	"log/slog"
	"testing"
	"time"
)

func TestSafeGoRecoversPanic(t *testing.T) {
	s := newTestServer()
	s.logger = slog.New(slog.NewTextHandler(io.Discard, nil))

	done := make(chan struct{})
	s.safeGo("test_panic", func() {
		defer close(done)
		panic("boom")
	})

	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("safeGo panic function did not finish")
	}
}

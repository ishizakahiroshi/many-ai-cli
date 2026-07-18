package wrapper

import (
	"bytes"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func TestWrapperSendWriteTimeout(t *testing.T) {
	cases := []struct {
		name string
		sec  int
		want time.Duration
	}{
		{name: "default", want: defaultWrapperSendWriteTimeout},
		{name: "configured", sec: 3, want: 3 * time.Second},
		{name: "non-positive uses default", sec: -1, want: defaultWrapperSendWriteTimeout},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			cfg := &config.Config{}
			cfg.Hub.WrapperSendWriteTimeoutSec = tc.sec
			if got := wrapperSendWriteTimeout(cfg); got != tc.want {
				t.Fatalf("wrapperSendWriteTimeout() = %s, want %s", got, tc.want)
			}
		})
	}
}

func TestPTYOutputWriterQueueOverflowDoesNotBlock(t *testing.T) {
	failure := make(chan struct{}, 1)
	writer := newPTYOutputWriter(&wrapperSession{currentSID: 1}, 1, func() {
		failure <- struct{}{}
	})

	writer.enqueue([]byte("first"))
	started := time.Now()
	writer.enqueue([]byte("second"))
	if elapsed := time.Since(started); elapsed > 100*time.Millisecond {
		t.Fatalf("queue overflow blocked enqueue for %s", elapsed)
	}

	select {
	case <-failure:
	case <-time.After(time.Second):
		t.Fatal("queue overflow did not trigger transport failure")
	}

	writer.mu.Lock()
	accepting := writer.accepting
	writer.mu.Unlock()
	if accepting {
		t.Fatal("writer continued accepting output after queue overflow")
	}

	// Start the consumer only after the overflow so the first chunk remains
	// queued and can be discarded as stale output after the fault.
	go writer.run()
	writer.finish()
}

func TestPTYOutputWriterResumeDropsStaleQueue(t *testing.T) {
	writer := newPTYOutputWriter(&wrapperSession{currentSID: 1}, 1, nil)
	writer.enqueue([]byte("stale"))
	writer.enqueue([]byte("overflow"))
	writer.resume()
	writer.enqueue([]byte("fresh"))

	got := <-writer.outCh
	if !bytes.Equal(got, []byte("fresh")) {
		t.Fatalf("resumed queue item = %q, want fresh", got)
	}
	select {
	case extra := <-writer.outCh:
		t.Fatalf("stale output remained queued: %q", extra)
	default:
	}

	go writer.run()
	writer.finish()
}

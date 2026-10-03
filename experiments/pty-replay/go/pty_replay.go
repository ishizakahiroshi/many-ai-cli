package main

import (
	"bytes"
	"sync"
)

// ptyReplayBuffer survives Hub restarts along with the wrapped CLI. Snapshot
// bytes and their cumulative offset must be captured under the same lock.
type ptyReplayBuffer struct {
	mu    sync.Mutex
	buf   bytes.Buffer
	total int64
}

func (r *ptyReplayBuffer) append(chunk []byte) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.total += int64(len(chunk))
	if len(chunk) >= ptyReplayBufferLimit {
		r.buf.Reset()
		chunk = chunk[len(chunk)-ptyReplayBufferLimit:]
	}
	r.buf.Write(chunk)
	if r.buf.Len() > ptyReplayBufferLimit {
		r.buf.Next(r.buf.Len() - ptyReplayBufferLimit)
	}
}

func (r *ptyReplayBuffer) snapshot() ([]byte, int64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	return bytes.Clone(r.buf.Bytes()), r.total
}


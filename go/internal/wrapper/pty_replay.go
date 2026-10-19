package wrapper

import (
	"bytes"
	"sync"

	"many-ai-cli/internal/proto"
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
	if len(chunk) >= proto.PTYReplayBufferLimit {
		r.buf.Reset()
		chunk = chunk[len(chunk)-proto.PTYReplayBufferLimit:]
	}
	r.buf.Write(chunk)
	if r.buf.Len() > proto.PTYReplayBufferLimit {
		r.buf.Next(r.buf.Len() - proto.PTYReplayBufferLimit)
	}
}

func (r *ptyReplayBuffer) snapshot() ([]byte, int64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	return bytes.Clone(r.buf.Bytes()), r.total
}

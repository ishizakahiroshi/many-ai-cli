package wrapper

import (
	"bytes"
	"strconv"
	"testing"

	"many-ai-cli/internal/proto"
)

func TestPTYReplayRetainsHistoryBeyond64KiB(t *testing.T) {
	var replay ptyReplayBuffer
	full := bytes.Repeat([]byte("previous conversation\r\n"), 24000)
	for offset := 0; offset < len(full); offset += 4096 {
		replay.append(full[offset:min(offset+4096, len(full))])
	}
	got, total := replay.snapshot()
	if total != int64(len(full)) || !bytes.Equal(got, full) {
		t.Fatalf("retained %d of %d bytes (total %d)", len(got), len(full), total)
	}
	got[0] = '!'
	next, _ := replay.snapshot()
	if !bytes.Equal(next, full) {
		t.Fatal("snapshot aliases retained bytes")
	}
}

func TestPTYReplayBoundsHistoryAndPreservesTotal(t *testing.T) {
	for _, chunkSize := range []int{4096, proto.PTYReplayBufferLimit + 123} {
		t.Run(strconv.Itoa(chunkSize), func(t *testing.T) {
			var replay ptyReplayBuffer
			full := make([]byte, proto.PTYReplayBufferLimit+32769)
			for i := range full {
				full[i] = byte(i % 251)
			}
			for offset := 0; offset < len(full); offset += chunkSize {
				replay.append(full[offset:min(offset+chunkSize, len(full))])
			}
			got, total := replay.snapshot()
			if total != int64(len(full)) || !bytes.Equal(got, full[len(full)-proto.PTYReplayBufferLimit:]) {
				t.Fatalf("tail or offset lost: got %d bytes, total %d", len(got), total)
			}
		})
	}
}

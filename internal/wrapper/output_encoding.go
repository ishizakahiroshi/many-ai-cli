package wrapper

import "unicode/utf8"

// pumpChunk merges any leftover bytes from the previous read (prev) with the
// new chunk, detects an incomplete UTF-8 sequence at the tail of the combined
// data, and returns:
//   - out: bytes safe to pass to repairMojibakeUTF8 and send to Hub
//   - carry: incomplete trailing bytes to prepend to the next chunk
//
// This prevents multi-byte characters that straddle a 4096-byte read boundary
// from being silently dropped or misrepaired by repairMojibakeUTF8 (which
// returns data unchanged when it contains an invalid sequence).
func pumpChunk(prev, chunk []byte) (out, carry []byte) {
	combined := append(prev, chunk...) //nolint:gocritic // intentional concat into new slice
	if len(combined) == 0 {
		return nil, nil
	}
	// Walk backwards to find the start of any incomplete multi-byte sequence.
	splitAt := len(combined)
	for i := len(combined) - 1; i >= 0 && i > len(combined)-4; i-- {
		b := combined[i]
		if b < 0x80 {
			// ASCII byte — everything from here onward is complete.
			break
		}
		if b >= 0xC0 {
			// This byte starts a multi-byte sequence. Check if the sequence
			// extends to the end without being complete.
			r, size := utf8.DecodeRune(combined[i:])
			if r == utf8.RuneError && size == 1 {
				// Incomplete sequence: carry these bytes to the next chunk.
				splitAt = i
			}
			break
		}
		// Continuation byte (0x80–0xBF): keep scanning backward.
	}
	return combined[:splitAt], combined[splitAt:]
}

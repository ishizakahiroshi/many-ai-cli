package hub

import "testing"

// TestVTBufferIgnoresDCSSequence verifies that a DCS (\x1bP ... ST) payload such
// as Sixel graphics is skipped instead of being rendered as literal screen text.
func TestVTBufferIgnoresDCSSequence(t *testing.T) {
	vt := newVTBuffer(40, 3)
	vt.Write([]byte("ok\x1bPq#0;2;0;0;0#0~~@@vv@@~~$\x1b\\!"))

	if got := vt.Lines()[0]; got != "ok!" {
		t.Fatalf("line 0 = %q, want ok!", got)
	}
}

// TestVTBufferIgnoresStringSequencesBELTerminated verifies APC/PM/SOS payloads
// terminated by BEL are skipped too (BEL is a valid String Terminator here).
func TestVTBufferIgnoresStringSequencesBELTerminated(t *testing.T) {
	cases := map[string]byte{
		"APC": '_',
		"PM":  '^',
		"SOS": 'X',
	}
	for name, intro := range cases {
		t.Run(name, func(t *testing.T) {
			vt := newVTBuffer(40, 3)
			seq := append([]byte("ok\x1b"), intro)
			seq = append(seq, []byte("garbage-payload-1234\x07!")...)
			vt.Write(seq)

			if got := vt.Lines()[0]; got != "ok!" {
				t.Fatalf("line 0 = %q, want ok!", got)
			}
		})
	}
}

// TestVTBufferIgnoresStringSequenceSplitWrites verifies the string sequence and
// its terminator may arrive across separate Write calls without leaking payload.
func TestVTBufferIgnoresStringSequenceSplitWrites(t *testing.T) {
	vt := newVTBuffer(40, 3)
	vt.Write([]byte("ok\x1bPq#0;2;0;0;0"))
	vt.Write([]byte("#0~~@@vv"))
	vt.Write([]byte("\x1b\\done"))

	if got := vt.Lines()[0]; got != "okdone" {
		t.Fatalf("line 0 = %q, want okdone", got)
	}
}

// TestVTBufferStringIntroducerHelper documents which introducers route through
// the string-skipping path (OSC, DCS, SOS, PM, APC) versus 2-byte escapes.
func TestVTBufferStringIntroducerHelper(t *testing.T) {
	stringIntroducers := []byte{']', 'P', 'X', '^', '_'}
	for _, c := range stringIntroducers {
		if !isStringSequenceIntroducer(c) {
			t.Fatalf("isStringSequenceIntroducer(%q) = false, want true", c)
		}
	}
	// Common non-string escapes must NOT be treated as string introducers.
	nonString := []byte{'[', '7', '8', 'M', 'D', 'c'}
	for _, c := range nonString {
		if isStringSequenceIntroducer(c) {
			t.Fatalf("isStringSequenceIntroducer(%q) = true, want false", c)
		}
	}
}

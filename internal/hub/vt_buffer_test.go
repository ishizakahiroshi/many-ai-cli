package hub

import "testing"

func TestVTBufferCursorAndErase(t *testing.T) {
	vt := newVTBuffer(20, 5)
	vt.Write([]byte("hello\nworld"))
	vt.Write([]byte("\x1b[2;1Hoverwrite\x1b[K"))

	lines := vt.Lines()
	if got := lines[0]; got != "hello" {
		t.Fatalf("line 0 = %q", got)
	}
	if got := lines[1]; got != "overwrite" {
		t.Fatalf("line 1 = %q", got)
	}
}

func TestVTBufferClearScreen(t *testing.T) {
	vt := newVTBuffer(20, 5)
	vt.Write([]byte("old\ncontent"))
	vt.Write([]byte("\x1b[2J\x1b[Hnew"))

	lines := vt.Lines()
	if got := lines[0]; got != "new" {
		t.Fatalf("line 0 = %q", got)
	}
	for i, line := range lines[1:] {
		if line != "" {
			t.Fatalf("line %d = %q, want empty", i+1, line)
		}
	}
}

func TestVTBufferSplitUTF8(t *testing.T) {
	vt := newVTBuffer(20, 5)
	vt.Write([]byte{0xe3, 0x81})
	vt.Write([]byte{0x82})
	if got := vt.Lines()[0]; got != "あ" {
		t.Fatalf("line 0 = %q", got)
	}
}

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

func TestVTBufferScrollsOnOverflow(t *testing.T) {
	vt := newVTBuffer(20, 3)
	vt.Write([]byte("one\r\ntwo\r\nthree\r\nfour"))

	lines := vt.Lines()
	want := []string{"two", "three", "four"}
	for i := range want {
		if lines[i] != want[i] {
			t.Fatalf("line %d = %q, want %q; lines=%#v", i, lines[i], want[i], lines)
		}
	}
}

func TestVTBufferResizePreservesVisibleCells(t *testing.T) {
	vt := newVTBuffer(8, 3)
	vt.Write([]byte("abcdef\r\nsecond"))
	vt.Resize(4, 2)

	lines := vt.Lines()
	if lines[0] != "abcd" {
		t.Fatalf("line 0 = %q, want abcd", lines[0])
	}
	if lines[1] != "seco" {
		t.Fatalf("line 1 = %q, want seco", lines[1])
	}
}

func TestVTBufferSaveRestoreCursor(t *testing.T) {
	vt := newVTBuffer(20, 3)
	vt.Write([]byte("a\x1b7\x1b[2;1Hb\x1b8c"))

	lines := vt.Lines()
	if lines[0] != "ac" {
		t.Fatalf("line 0 = %q, want ac", lines[0])
	}
	if lines[1] != "b" {
		t.Fatalf("line 1 = %q, want b", lines[1])
	}
}

func TestVTBufferIgnoresOSCSequences(t *testing.T) {
	vt := newVTBuffer(20, 3)
	vt.Write([]byte("ok\x1b]0;window title\x07!"))

	if got := vt.Lines()[0]; got != "ok!" {
		t.Fatalf("line 0 = %q, want ok!", got)
	}
}

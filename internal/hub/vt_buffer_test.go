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

// TestVTBufferCapsUnterminatedEscape は、未終端 CSI（ESC [ の後に最終バイト
// 0x40-0x7e が来ないパラメータバイト列）を大量に送っても内部エスケープバッファ
// b.esc が無制限に増加しないことを検証する。悪意ある/プロンプトインジェクションされた
// AI 出力によるメモリ枯渇 DoS の回帰防止（AUDIT-7）。バイト列は複数 Write（＝WS
// フレーム相当）にまたがって送り、フレーム跨ぎの累積も cap されることを確認する。
func TestVTBufferCapsUnterminatedEscape(t *testing.T) {
	vt := newVTBuffer(20, 5)
	vt.Write([]byte("\x1b[")) // CSI 導入（未終端）
	for i := 0; i < 100; i++ {
		vt.Write([]byte("1234567890")) // 計 1000 パラメータバイトを複数 Write で送る
	}
	if len(vt.esc) > maxEscapeSeqBytes {
		t.Fatalf("unterminated CSI grew b.esc to %d bytes, want <= %d", len(vt.esc), maxEscapeSeqBytes)
	}
}

// TestVTBufferCapsUnterminatedOSC は、終端子（BEL / ST）を送らない OSC 導入子
// （ESC ]）を延々と流しても、maxStringSeqSkipBytes を超えたらスキップが打ち切られ、
// inOSC が false に戻って承認検知が回復することを検証する（AUDIT-9・検知盲目化の防止）。
func TestVTBufferCapsUnterminatedOSC(t *testing.T) {
	vt := newVTBuffer(20, 5)
	vt.Write([]byte("\x1b]")) // OSC 導入（未終端）
	chunk := make([]byte, 4096)
	for i := range chunk {
		chunk[i] = 'x'
	}
	for written := 0; written <= maxStringSeqSkipBytes; written += len(chunk) {
		vt.Write(chunk)
	}
	if vt.inOSC {
		t.Fatal("unterminated OSC left inOSC=true (approval detection permanently blinded)")
	}
	if vt.stringSeqSkipBytes > maxStringSeqSkipBytes {
		t.Fatalf("stringSeqSkipBytes = %d, want <= %d", vt.stringSeqSkipBytes, maxStringSeqSkipBytes)
	}
	// 打ち切り後は通常の描画が再開する（検知が回復）。
	vt.Write([]byte("\r\nrecovered"))
	found := false
	for _, line := range vt.Lines() {
		if line == "recovered" {
			found = true
			break
		}
	}
	if !found {
		t.Fatal("text not rendered after OSC abort (VT did not recover)")
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

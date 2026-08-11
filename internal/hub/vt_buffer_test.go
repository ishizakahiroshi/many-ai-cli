package hub

import (
	"fmt"
	"testing"
)

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

// TestVTBufferResizeDropsScrollback は、サイズが変わる resize で scrollback を捨てる
// ことを検証する。このミラーは実端末と違い reflow しないため、resize 前に押し出された
// 行は旧サイズの残骸でしかない。TUI は SIGWINCH で画面全体を描き直すので同じ内容が
// 新しい世代として必ず流れてくる。残すと承認マーカーの開始行が新旧 2 本同居する。
func TestVTBufferResizeDropsScrollback(t *testing.T) {
	vt := newVTBuffer(40, 3)
	for i := 0; i < 10; i++ {
		vt.Write([]byte(fmt.Sprintf("line%d\r\n", i)))
	}
	if len(vt.scrollback) == 0 {
		t.Fatal("scrollback did not grow; test setup is wrong")
	}
	vt.Resize(40, 12)
	if len(vt.scrollback) != 0 {
		t.Fatalf("scrollback = %d lines after resize, want 0", len(vt.scrollback))
	}
}

// TestVTBufferSameSizeResizeKeepsScrollback は、同一サイズの Resize では履歴を捨てない
// ことを検証する。同じサイズなら TUI へ SIGWINCH は届かず再描画も起きないため、捨てると
// 画面高を超える承認ブロック（Grok の複数質問等）を取りこぼすだけになる。
func TestVTBufferSameSizeResizeKeepsScrollback(t *testing.T) {
	vt := newVTBuffer(40, 3)
	for i := 0; i < 10; i++ {
		vt.Write([]byte(fmt.Sprintf("line%d\r\n", i)))
	}
	before := len(vt.scrollback)
	vt.Resize(40, 3)
	if len(vt.scrollback) != before {
		t.Fatalf("scrollback = %d lines, want %d", len(vt.scrollback), before)
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

func TestVTBufferScrollbackOnPushOut(t *testing.T) {
	vt := newVTBuffer(20, 3)
	// 3 行画面: 5 行流すと先頭 2 行が scrollback へ
	vt.Write([]byte("A0\r\nB1\r\nC2\r\nD3\r\nE4"))
	if len(vt.scrollback) != 2 {
		t.Fatalf("scrollback len = %d, want 2; scrollback=%v lines=%v", len(vt.scrollback), vt.scrollback, vt.Lines())
	}
	if vt.scrollback[0] != "A0" || vt.scrollback[1] != "B1" {
		t.Fatalf("scrollback = %v, want [A0 B1]", vt.scrollback)
	}
	got := vt.TailLinesWithScrollback(5)
	if len(got) != 5 {
		t.Fatalf("TailLinesWithScrollback(5) len = %d, want 5; got=%v", len(got), got)
	}
	want := []string{"A0", "B1", "C2", "D3", "E4"}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("got[%d]=%q want %q (full=%v)", i, got[i], want[i], got)
		}
	}
	// n が画面内なら scrollback を混ぜない
	screenOnly := vt.TailLinesWithScrollback(2)
	if len(screenOnly) != 2 || screenOnly[0] != "D3" || screenOnly[1] != "E4" {
		t.Fatalf("screen-only tail = %v", screenOnly)
	}
}

func TestVTBufferScrollbackNotFilledOnClear(t *testing.T) {
	vt := newVTBuffer(20, 3)
	vt.Write([]byte("A0\r\nB1\r\nC2\r\nD3"))
	if len(vt.scrollback) == 0 {
		t.Fatal("expected scrollback before clear")
	}
	before := append([]string(nil), vt.scrollback...)
	vt.Write([]byte("\x1b[2J\x1b[H"))
	if len(vt.scrollback) != len(before) {
		t.Fatalf("scrollback changed on clear: before=%v after=%v", before, vt.scrollback)
	}
	for i := range before {
		if vt.scrollback[i] != before[i] {
			t.Fatalf("scrollback[%d] changed on clear", i)
		}
	}
	vt.Write([]byte("new"))
	if got := vt.Lines()[0]; got != "new" {
		t.Fatalf("line0 after clear = %q", got)
	}
}

func TestVTBufferScrollbackAdjacentDedupeEmptyOnly(t *testing.T) {
	vt := newVTBuffer(10, 2)
	// 非空の同一行は保持する（マーカー本文の連続同一行を壊さない）
	vt.Write([]byte("DUP\r\nX\r\nDUP\r\nDUP\r\nY"))
	dupCount := 0
	for _, line := range vt.scrollback {
		if line == "DUP" {
			dupCount++
		}
	}
	if dupCount < 2 {
		t.Fatalf("expected non-empty DUP lines kept in scrollback, got %v", vt.scrollback)
	}

	// 空行の隣接重複だけ潰す
	vt2 := newVTBuffer(10, 2)
	vt2.Write([]byte("A\r\n\r\n\r\n\r\nB\r\nC"))
	emptyRun := 0
	maxEmptyRun := 0
	for _, line := range vt2.scrollback {
		if line == "" {
			emptyRun++
			if emptyRun > maxEmptyRun {
				maxEmptyRun = emptyRun
			}
		} else {
			emptyRun = 0
		}
	}
	if maxEmptyRun > 1 {
		t.Fatalf("adjacent empty lines should be collapsed, scrollback=%v", vt2.scrollback)
	}
}

func TestVTBufferScrollbackCap(t *testing.T) {
	vt := newVTBuffer(8, 2)
	for i := 0; i < maxVTScrollbackLines+50; i++ {
		vt.Write([]byte(fmt.Sprintf("L%04d\r\n", i)))
	}
	if len(vt.scrollback) > maxVTScrollbackLines {
		t.Fatalf("scrollback len = %d, want <= %d", len(vt.scrollback), maxVTScrollbackLines)
	}
	if len(vt.scrollback) < maxVTScrollbackLines {
		t.Fatalf("scrollback len = %d, want %d (should be full)", len(vt.scrollback), maxVTScrollbackLines)
	}
}

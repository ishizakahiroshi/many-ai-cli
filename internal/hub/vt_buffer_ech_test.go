package hub

import (
	"fmt"
	"strings"
	"testing"
)

// ECH（CSI n X）: カーソル位置から n セルを空白にし、カーソルは動かさない。
// bugfix_vt-mirror-ech-unimplemented-marker-leak_2026-09-07.md

func TestVTBufferEraseCharsClearsCellsAndKeepsCursor(t *testing.T) {
	vt := newVTBuffer(10, 2)
	vt.Write([]byte("abcdefghij"))
	vt.Write([]byte("\x1b[1;3H\x1b[3X"))
	if got := strings.TrimRight(vt.Lines()[0], " "); got != "ab   fghij" {
		t.Fatalf("after ECH 3: %q", got)
	}
	// カーソルは消去開始位置（3 桁目）に残る
	vt.Write([]byte("Z"))
	if got := strings.TrimRight(vt.Lines()[0], " "); got != "abZ  fghij" {
		t.Fatalf("after writing at cursor: %q", got)
	}
}

func TestVTBufferEraseCharsDefaultsToOneAndClampsAtRowEnd(t *testing.T) {
	vt := newVTBuffer(5, 1)
	vt.Write([]byte("abcde\x1b[1;1H\x1b[X"))
	if got := vt.Lines()[0]; got != " bcde" {
		t.Fatalf("default n=1: %q", got)
	}
	vt.Write([]byte("\x1b[1;4H\x1b[99X"))
	// Lines() は行末の空白を落として返すので、消えた 2 セルは末尾の欠落として見える
	if got := vt.Lines()[0]; got != " bc" {
		t.Fatalf("clamped at row end: %q", got)
	}
	if vt.cells[0][3] != ' ' || vt.cells[0][4] != ' ' {
		t.Fatalf("cells 4-5 not cleared: %q %q", vt.cells[0][3], vt.cells[0][4])
	}
	if vt.row != 0 || vt.col != 3 {
		t.Fatalf("cursor moved to %d;%d, want 1;4", vt.row+1, vt.col+1)
	}
}

func TestVTBufferEraseCharsInvalidatesWideRunePair(t *testing.T) {
	vt := newVTBuffer(10, 1)
	vt.Write([]byte("日本"))
	// 「日」の右半分だけを消しても、実端末と同じく対ごと空白になる
	vt.Write([]byte("\x1b[1;2H\x1b[1X"))
	if vt.cells[0][0] != ' ' || vt.cells[0][1] != ' ' {
		t.Fatalf("wide pair not invalidated: %q %q", vt.cells[0][0], vt.cells[0][1])
	}
	if vt.cells[0][2] != '本' {
		t.Fatalf("neighbouring wide rune damaged: %q", vt.cells[0][2])
	}
}

// TUI が短くなった新フレームを描いたあと、旧フレームの余った行を ECH で消す。
// ミラーが ECH を無視すると旧 CLOSE がその行に残り、抽出が「最後の CLOSE」として拾って
// ブロックに CLOSE が 2 つ入り marker_leak になる（approval-corrupt ダンプ 2026-09-01 #1 と
// 同じ形）。対照として、消去列が無ければ同じ入力が marker_leak になることも固定する。
func TestApprovalMarkerNotLeakedWhenTUIErasesOldFrameWithECH(t *testing.T) {
	draw := func(vt *vtBuffer, top int, lines []string) {
		for i, l := range lines {
			vt.Write([]byte(fmt.Sprintf("\x1b[%d;1H%s\x1b[K", top+i, l)))
		}
	}
	older := []string{
		approvalMarkerOpen,
		"Q1 scope?",
		" 1. Yes (Recommended)",
		" 2. No",
		" N. User specifies",
		approvalMarkerClose,
	}
	newer := []string{
		approvalMarkerOpen,
		"Q1 scope?",
		" 1. Yes (Recommended)",
		" N. User specifies",
		approvalMarkerClose,
	}
	for _, tc := range []struct {
		name  string
		erase string
		want  string
	}{
		{name: "tui erases leftover row with ECH", erase: "\x1b[7;1H\x1b[40X", want: ""},
		{name: "control: leftover row not erased", erase: "", want: "marker_leak"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			vt := newVTBuffer(40, 10)
			draw(vt, 2, older)
			draw(vt, 2, newer)
			vt.Write([]byte(tc.erase))
			marker := extractApprovalMarkerBlockFromVT(vt)
			if marker == nil {
				t.Fatal("no marker block extracted")
			}
			if got := classifyApprovalMarkerBlock(marker.Block); got != tc.want {
				t.Fatalf("verdict = %q, want %q\nblock:\n%s", got, tc.want, marker.Block)
			}
			if tc.want == "" && strings.Count(marker.Block, approvalMarkerClose) != 1 {
				t.Fatalf("block must contain exactly one CLOSE:\n%s", marker.Block)
			}
		})
	}
}

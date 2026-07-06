package hub

import (
	"strconv"
	"strings"
	"unicode/utf8"
)

const (
	defaultVTCols = 200
	defaultVTRows = 50
)

type vtBuffer struct {
	cols int
	rows int

	cells [][]rune
	row   int
	col   int

	savedRow int
	savedCol int

	utf8Pending []byte
	esc         []byte
	inOSC       bool
	// inStringSeq は OSC 以外の文字列シーケンス（DCS/SOS/PM/APC）の
	// ペイロードをスキップ中かを示す（finding #: DCS Sixel 等を画面に出力しない）。
	inStringSeq bool
}

// isStringSequenceIntroducer は ESC の次のバイトが文字列シーケンス（OSC/DCS/SOS/PM/APC）を
// 導入するかを返す。これらはペイロード全体を ST（ESC \）または BEL で終端されるまでスキップする。
//   - ']' = OSC (Operating System Command)
//   - 'P' = DCS (Device Control String)
//   - 'X' = SOS (Start of String)
//   - '^' = PM  (Privacy Message)
//   - '_' = APC (Application Program Command)
func isStringSequenceIntroducer(c byte) bool {
	switch c {
	case ']', 'P', 'X', '^', '_':
		return true
	}
	return false
}

func newVTBuffer(cols, rows int) *vtBuffer {
	if cols <= 0 {
		cols = defaultVTCols
	}
	if rows <= 0 {
		rows = defaultVTRows
	}
	b := &vtBuffer{}
	b.Resize(cols, rows)
	return b
}

func (b *vtBuffer) Reset() {
	cols, rows := b.cols, b.rows
	*b = *newVTBuffer(cols, rows)
}

func (b *vtBuffer) Resize(cols, rows int) {
	if cols <= 0 {
		cols = defaultVTCols
	}
	if rows <= 0 {
		rows = defaultVTRows
	}
	next := make([][]rune, rows)
	for r := 0; r < rows; r++ {
		next[r] = make([]rune, cols)
		for c := range next[r] {
			next[r][c] = ' '
		}
	}
	if b.cells != nil {
		copyRows := min(rows, b.rows)
		copyCols := min(cols, b.cols)
		for r := 0; r < copyRows; r++ {
			copy(next[r][:copyCols], b.cells[r][:copyCols])
		}
	}
	b.cols = cols
	b.rows = rows
	b.cells = next
	b.row = clampInt(b.row, 0, rows-1)
	b.col = clampInt(b.col, 0, cols-1)
	b.savedRow = clampInt(b.savedRow, 0, rows-1)
	b.savedCol = clampInt(b.savedCol, 0, cols-1)
}

func (b *vtBuffer) Write(data []byte) {
	if len(data) == 0 {
		return
	}
	buf := data
	if len(b.utf8Pending) > 0 {
		combined := make([]byte, 0, len(b.utf8Pending)+len(data))
		combined = append(combined, b.utf8Pending...)
		combined = append(combined, data...)
		buf = combined
		b.utf8Pending = nil
	}
	for len(buf) > 0 {
		// OSC (ESC ]) スキップ: BEL または ST (ESC \) で終端。
		if b.inOSC {
			if buf[0] == 0x07 {
				b.inOSC = false
				buf = buf[1:]
				continue
			}
			if len(buf) >= 2 && buf[0] == 0x1b && buf[1] == '\\' {
				b.inOSC = false
				buf = buf[2:]
				continue
			}
			buf = buf[1:]
			continue
		}
		// DCS/SOS/PM/APC (ESC P/X/^/_) スキップ: ST (ESC \) または BEL で終端。
		if b.inStringSeq {
			if buf[0] == 0x07 {
				b.inStringSeq = false
				buf = buf[1:]
				continue
			}
			if len(buf) >= 2 && buf[0] == 0x1b && buf[1] == '\\' {
				b.inStringSeq = false
				buf = buf[2:]
				continue
			}
			buf = buf[1:]
			continue
		}
		if len(b.esc) > 0 {
			b.esc = append(b.esc, buf[0])
			buf = buf[1:]
			if b.esc[0] == 0x1b && len(b.esc) >= 2 && b.esc[1] == ']' {
				b.esc = nil
				b.inOSC = true
				continue
			}
			// DCS/SOS/PM/APC 導入文字の検出
			if b.esc[0] == 0x1b && len(b.esc) >= 2 && isStringSequenceIntroducer(b.esc[1]) {
				b.esc = nil
				b.inStringSeq = true
				continue
			}
			if b.escapeComplete() {
				b.processEscape(string(b.esc))
				b.esc = nil
			}
			continue
		}

		if buf[0] == 0x1b {
			b.esc = []byte{0x1b}
			buf = buf[1:]
			continue
		}
		r, size := utf8.DecodeRune(buf)
		if r == utf8.RuneError && size == 1 && !utf8.FullRune(buf) {
			b.utf8Pending = append(b.utf8Pending[:0], buf...)
			return
		}
		b.writeRune(r)
		buf = buf[size:]
	}
}

func (b *vtBuffer) Lines() []string {
	lines := make([]string, 0, b.rows)
	for r := 0; r < b.rows; r++ {
		lines = append(lines, strings.TrimRight(string(b.cells[r]), " "))
	}
	return lines
}

func (b *vtBuffer) TailLines(n int) []string {
	lines := b.Lines()
	if n <= 0 || n >= len(lines) {
		return lines
	}
	return lines[len(lines)-n:]
}

func (b *vtBuffer) escapeComplete() bool {
	if len(b.esc) < 2 {
		return false
	}
	if b.esc[1] != '[' {
		return true
	}
	if len(b.esc) < 3 {
		return false
	}
	ch := b.esc[len(b.esc)-1]
	return ch >= 0x40 && ch <= 0x7e
}

func (b *vtBuffer) processEscape(seq string) {
	if seq == "\x1b7" {
		b.savedRow, b.savedCol = b.row, b.col
		return
	}
	if seq == "\x1b8" {
		b.row, b.col = b.savedRow, b.savedCol
		return
	}
	if !strings.HasPrefix(seq, "\x1b[") || len(seq) < 3 {
		return
	}
	final := seq[len(seq)-1]
	body := seq[2 : len(seq)-1]
	private := strings.HasPrefix(body, "?")
	if private {
		body = strings.TrimPrefix(body, "?")
	}
	params := parseCSIParams(body)
	p := func(idx, def int) int {
		if idx >= len(params) || params[idx] == 0 {
			return def
		}
		return params[idx]
	}

	switch final {
	case 'A':
		b.row = clampInt(b.row-p(0, 1), 0, b.rows-1)
	case 'B':
		b.row = clampInt(b.row+p(0, 1), 0, b.rows-1)
	case 'C':
		b.col = clampInt(b.col+p(0, 1), 0, b.cols-1)
	case 'D':
		b.col = clampInt(b.col-p(0, 1), 0, b.cols-1)
	case 'G':
		b.col = clampInt(p(0, 1)-1, 0, b.cols-1)
	case 'H', 'f':
		b.row = clampInt(p(0, 1)-1, 0, b.rows-1)
		b.col = clampInt(p(1, 1)-1, 0, b.cols-1)
	case 'J':
		b.eraseDisplay(p(0, 0))
	case 'K':
		b.eraseLine(p(0, 0))
	case 's':
		b.savedRow, b.savedCol = b.row, b.col
	case 'u':
		b.row, b.col = b.savedRow, b.savedCol
	case 'h', 'l':
		if private && len(params) > 0 && params[0] == 1049 {
			b.clearAll()
			b.row, b.col = 0, 0
		}
	}
}

func parseCSIParams(body string) []int {
	if body == "" {
		return nil
	}
	parts := strings.Split(body, ";")
	out := make([]int, 0, len(parts))
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if part == "" {
			out = append(out, 0)
			continue
		}
		n, err := strconv.Atoi(part)
		if err != nil {
			out = append(out, 0)
			continue
		}
		out = append(out, n)
	}
	return out
}

func (b *vtBuffer) writeRune(r rune) {
	switch r {
	case '\r':
		b.col = 0
	case '\n':
		b.newLine()
	case '\b':
		if b.col > 0 {
			b.col--
		}
	case '\t':
		next := ((b.col / 8) + 1) * 8
		b.col = clampInt(next, 0, b.cols-1)
	default:
		if r < 0x20 {
			return
		}
		b.cells[b.row][b.col] = r
		b.col++
		if b.col >= b.cols {
			b.col = 0
			b.newLine()
		}
	}
}

func (b *vtBuffer) newLine() {
	b.row++
	if b.row < b.rows {
		return
	}
	copy(b.cells, b.cells[1:])
	b.cells[b.rows-1] = make([]rune, b.cols)
	for c := range b.cells[b.rows-1] {
		b.cells[b.rows-1][c] = ' '
	}
	b.row = b.rows - 1
}

func (b *vtBuffer) eraseDisplay(mode int) {
	switch mode {
	case 1:
		for r := 0; r < b.row; r++ {
			b.clearRow(r, 0, b.cols-1)
		}
		b.clearRow(b.row, 0, b.col)
	case 2, 3:
		b.clearAll()
		b.row, b.col = 0, 0
	default:
		b.clearRow(b.row, b.col, b.cols-1)
		for r := b.row + 1; r < b.rows; r++ {
			b.clearRow(r, 0, b.cols-1)
		}
	}
}

func (b *vtBuffer) eraseLine(mode int) {
	switch mode {
	case 1:
		b.clearRow(b.row, 0, b.col)
	case 2:
		b.clearRow(b.row, 0, b.cols-1)
	default:
		b.clearRow(b.row, b.col, b.cols-1)
	}
}

func (b *vtBuffer) clearAll() {
	for r := 0; r < b.rows; r++ {
		b.clearRow(r, 0, b.cols-1)
	}
}

func (b *vtBuffer) clearRow(row, start, end int) {
	if row < 0 || row >= b.rows {
		return
	}
	start = clampInt(start, 0, b.cols-1)
	end = clampInt(end, 0, b.cols-1)
	for c := start; c <= end; c++ {
		b.cells[row][c] = ' '
	}
}

func clampInt(v, lo, hi int) int {
	if hi < lo {
		return lo
	}
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}


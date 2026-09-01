package wrapper

import (
	"bytes"
	"fmt"
	"strconv"
)

// terminalQueryState carries an incomplete CSI prefix across PTY read chunks and
// tracks a best-effort cursor position for CPR replies (CSI 6n → CSI row;col R).
type terminalQueryState struct {
	carry []byte
	row   int
	col   int
}

func newTerminalQueryState() terminalQueryState {
	return terminalQueryState{row: 1, col: 1}
}

// processTerminalQueries scans PTY output for terminal device queries that a real
// emulator (VTE / ConPTY) would answer locally:
//   - DSR cursor position: CSI 6n  → reply CSI {row};{col}R
//   - DSR status:          CSI 5n  → reply CSI 0n
//   - Primary DA:          CSI c / CSI 0c → reply CSI ?1;2c
//
// Matched queries are stripped from the forwarded stream and their replies are
// returned for writing back to the PTY master. Incomplete ESC/[ prefixes are
// carried in state. Cursor tracking is best-effort (CUP/CHA/VPA/CU*/CR/LF/BS)
// so CPR is "reasonable" rather than emulator-perfect — enough for Ink/Claude
// Code to leave raw-mode standby on Linux creack/pty wraps.
func processTerminalQueries(state terminalQueryState, chunk []byte) (forward, replies []byte, next terminalQueryState) {
	combined := make([]byte, 0, len(state.carry)+len(chunk))
	combined = append(combined, state.carry...)
	combined = append(combined, chunk...)

	row, col := state.row, state.col
	if row < 1 {
		row = 1
	}
	if col < 1 {
		col = 1
	}

	out := make([]byte, 0, len(combined))
	var rep []byte
	i := 0
	for i < len(combined) {
		if combined[i] != 0x1b {
			b := combined[i]
			out = append(out, b)
			row, col = advanceCursorPlain(b, row, col)
			i++
			continue
		}
		if i+1 >= len(combined) {
			return out, rep, terminalQueryState{carry: append([]byte{}, combined[i:]...), row: row, col: col}
		}
		if combined[i+1] != '[' {
			out = append(out, combined[i], combined[i+1])
			i += 2
			continue
		}

		// CSI: parameter bytes 0x30–0x3F, intermediate 0x20–0x2F, final 0x40–0x7E.
		j := i + 2
		for j < len(combined) {
			b := combined[j]
			if b >= 0x40 && b <= 0x7e {
				params := combined[i+2 : j]
				final := b
				if reply, ok := terminalQueryReply(params, final, row, col); ok {
					rep = append(rep, reply...)
				} else {
					out = append(out, combined[i:j+1]...)
					row, col = applyCSICursor(params, final, row, col)
				}
				i = j + 1
				break
			}
			if b < 0x20 || b > 0x3f {
				// Not a well-formed CSI — forward ESC and rescan from next byte.
				out = append(out, combined[i])
				i++
				break
			}
			j++
		}
		if j >= len(combined) {
			return out, rep, terminalQueryState{carry: append([]byte{}, combined[i:]...), row: row, col: col}
		}
	}
	return out, rep, terminalQueryState{row: row, col: col}
}

func terminalQueryReply(params []byte, final byte, row, col int) ([]byte, bool) {
	switch final {
	case 'n':
		switch {
		case csiParamsEqual(params, "6"):
			return []byte(fmt.Sprintf("\x1b[%d;%dR", row, col)), true
		case csiParamsEqual(params, "5"):
			return []byte("\x1b[0n"), true
		}
	case 'c':
		// Primary DA (no params or 0). Ignore secondary DA (leading '>').
		if len(params) == 0 || csiParamsEqual(params, "0") {
			return []byte("\x1b[?1;2c"), true
		}
	}
	return nil, false
}

func csiParamsEqual(params []byte, want string) bool {
	return string(params) == want
}

func advanceCursorPlain(b byte, row, col int) (int, int) {
	switch b {
	case '\n':
		return row + 1, 1
	case '\r':
		return row, 1
	case '\b':
		if col > 1 {
			return row, col - 1
		}
		return row, 1
	default:
		if b >= 0x20 && b != 0x7f {
			return row, col + 1
		}
		return row, col
	}
}

func applyCSICursor(params []byte, final byte, row, col int) (int, int) {
	switch final {
	case 'H', 'f': // CUP / HVP
		r, c := parseCSIRowCol(params)
		return r, c
	case 'G': // CHA
		n := parseCSISingle(params, 1)
		if n < 1 {
			n = 1
		}
		return row, n
	case 'd': // VPA
		n := parseCSISingle(params, 1)
		if n < 1 {
			n = 1
		}
		return n, col
	case 'A': // CUU
		n := parseCSISingle(params, 1)
		row -= n
		if row < 1 {
			row = 1
		}
		return row, col
	case 'B': // CUD
		return row + parseCSISingle(params, 1), col
	case 'C': // CUF
		return row, col + parseCSISingle(params, 1)
	case 'D': // CUB
		n := parseCSISingle(params, 1)
		col -= n
		if col < 1 {
			col = 1
		}
		return row, col
	default:
		return row, col
	}
}

func parseCSIRowCol(params []byte) (row, col int) {
	row, col = 1, 1
	if len(params) == 0 {
		return row, col
	}
	parts := bytes.Split(params, []byte{';'})
	if len(parts) >= 1 && len(parts[0]) > 0 {
		if n, err := strconv.Atoi(string(parts[0])); err == nil && n > 0 {
			row = n
		}
	}
	if len(parts) >= 2 && len(parts[1]) > 0 {
		if n, err := strconv.Atoi(string(parts[1])); err == nil && n > 0 {
			col = n
		}
	}
	return row, col
}

func parseCSISingle(params []byte, def int) int {
	if len(params) == 0 {
		return def
	}
	n, err := strconv.Atoi(string(params))
	if err != nil || n < 1 {
		return def
	}
	return n
}

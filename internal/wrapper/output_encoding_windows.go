//go:build windows

package wrapper

import "unicode/utf8"

var windows1252Reverse = map[rune]byte{
	'\u20ac': 0x80,
	'\u201a': 0x82,
	'\u0192': 0x83,
	'\u201e': 0x84,
	'\u2026': 0x85,
	'\u2020': 0x86,
	'\u2021': 0x87,
	'\u02c6': 0x88,
	'\u2030': 0x89,
	'\u0160': 0x8a,
	'\u2039': 0x8b,
	'\u0152': 0x8c,
	'\u017d': 0x8e,
	'\u2018': 0x91,
	'\u2019': 0x92,
	'\u201c': 0x93,
	'\u201d': 0x94,
	'\u2022': 0x95,
	'\u2013': 0x96,
	'\u2014': 0x97,
	'\u02dc': 0x98,
	'\u2122': 0x99,
	'\u0161': 0x9a,
	'\u203a': 0x9b,
	'\u0153': 0x9c,
	'\u017e': 0x9e,
	'\u0178': 0x9f,
}

// repairMojibakeUTF8 reverses the common Windows path where UTF-8 output is
// decoded as Windows-1252 and then emitted as UTF-8 again (for example
// "ãƒ­ãƒ¼ã‚«ãƒ«" instead of "ローカル", or "â”€" instead of "─").
func repairMojibakeUTF8(data []byte) []byte {
	if len(data) == 0 || !utf8.Valid(data) {
		return data
	}
	text := string(data)
	before := mojibakeScore(text)
	if before == 0 {
		return data
	}
	raw, ok := encodeWindows1252(text)
	if !ok || !utf8.Valid(raw) {
		return data
	}
	repaired := string(raw)
	if mojibakeScore(repaired) >= before || repairedSignalScore(repaired) == 0 {
		return data
	}
	return raw
}

func encodeWindows1252(s string) ([]byte, bool) {
	out := make([]byte, 0, len(s))
	for _, r := range s {
		switch {
		case r <= 0x7f:
			out = append(out, byte(r))
		case r >= 0x80 && r <= 0x9f:
			out = append(out, byte(r))
		case r >= 0xa0 && r <= 0xff:
			out = append(out, byte(r))
		default:
			b, ok := windows1252Reverse[r]
			if !ok {
				return nil, false
			}
			out = append(out, b)
		}
	}
	return out, true
}

func mojibakeScore(s string) int {
	score := 0
	for _, r := range s {
		switch r {
		case 'Ã', 'ã', 'â', 'Â':
			score += 2
		case 'å', 'æ', 'ç':
			score++
		}
	}
	return score
}

func repairedSignalScore(s string) int {
	score := 0
	for _, r := range s {
		switch {
		case r >= 0x3040 && r <= 0x30ff: // Hiragana / Katakana
			score++
		case r >= 0x3400 && r <= 0x9fff: // CJK
			score++
		case r >= 0x2500 && r <= 0x257f: // box drawing
			score++
		case r >= 0x2190 && r <= 0x21ff: // arrows
			score++
		case r >= 0x2600 && r <= 0x27bf: // misc symbols / dingbats
			score++
		}
	}
	return score
}

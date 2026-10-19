//go:build !windows

package wrapper

import "testing"

func TestRepairMojibakeUTF8NoopOnNonWindows(t *testing.T) {
	// UTF-8 bytes of "ローカル", written as \x escapes so the source contains no
	// literal invisible character. (staticcheck ST1018 flags a raw U+00AD soft
	// hyphen, which the previous mojibake glyph literal contained.)
	in := []byte("\xe3\x83\xad\xe3\x83\xbc\xe3\x82\xab\xe3\x83\xab")
	got := repairMojibakeUTF8(in)
	if string(got) != string(in) {
		t.Fatalf("non-Windows repair should pass through: %q", string(got))
	}
}

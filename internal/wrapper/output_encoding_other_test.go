//go:build !windows

package wrapper

import "testing"

func TestRepairMojibakeUTF8NoopOnNonWindows(t *testing.T) {
	in := []byte("ãƒ­ãƒ¼ã‚«ãƒ«")
	got := repairMojibakeUTF8(in)
	if string(got) != string(in) {
		t.Fatalf("non-Windows repair should pass through: %q", string(got))
	}
}

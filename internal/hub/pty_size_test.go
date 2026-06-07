package hub

import "testing"

func TestUsableInitPTYSizeRejectsTinyValues(t *testing.T) {
	if cols, rows, ok := usableInitPTYSize(12, 8); ok || cols != 0 || rows != 0 {
		t.Fatalf("usableInitPTYSize(12, 8) = (%d, %d, %v), want rejected", cols, rows, ok)
	}
}

func TestUsableInitPTYSizeAcceptsNormalValues(t *testing.T) {
	cols, rows, ok := usableInitPTYSize(120, 30)
	if !ok || cols != 120 || rows != 30 {
		t.Fatalf("usableInitPTYSize(120, 30) = (%d, %d, %v), want accepted", cols, rows, ok)
	}
}

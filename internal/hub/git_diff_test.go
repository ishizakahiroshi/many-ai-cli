package hub

import (
	"strings"
	"testing"
)

func TestSynthesizeAddDiffText(t *testing.T) {
	diff, added := synthesizeAddDiff("docs/sample.md", []byte("line1\nline2\n"))
	if added != 2 {
		t.Fatalf("added = %d, want 2", added)
	}
	wants := []string{
		"diff --git a/docs/sample.md b/docs/sample.md",
		"new file mode 100644",
		"--- /dev/null",
		"+++ b/docs/sample.md",
		"@@ -0,0 +1,2 @@",
		"+line1",
		"+line2",
	}
	for _, w := range wants {
		if !strings.Contains(diff, w) {
			t.Errorf("diff missing %q\n---\n%s", w, diff)
		}
	}
	if strings.Contains(diff, "No newline at end of file") {
		t.Errorf("unexpected no-newline marker for trailing-newline content")
	}
}

func TestSynthesizeAddDiffNoTrailingNewline(t *testing.T) {
	diff, added := synthesizeAddDiff("a.txt", []byte("only"))
	if added != 1 {
		t.Fatalf("added = %d, want 1", added)
	}
	if !strings.Contains(diff, "@@ -0,0 +1,1 @@\n+only\n\\ No newline at end of file\n") {
		t.Errorf("no-newline marker missing:\n%s", diff)
	}
}

func TestSynthesizeAddDiffEmpty(t *testing.T) {
	diff, added := synthesizeAddDiff("empty.txt", nil)
	if added != 0 {
		t.Fatalf("added = %d, want 0", added)
	}
	if strings.Contains(diff, "@@") {
		t.Errorf("empty file should have no hunk:\n%s", diff)
	}
	if !strings.Contains(diff, "+++ b/empty.txt") {
		t.Errorf("header missing:\n%s", diff)
	}
}

func TestSynthesizeAddDiffBinary(t *testing.T) {
	diff, added := synthesizeAddDiff("bin.dat", []byte{0x89, 0x50, 0x00, 0x0a, 0x41})
	if added != 0 {
		t.Fatalf("added = %d, want 0 for binary", added)
	}
	if !strings.Contains(diff, "Binary file (new)") {
		t.Errorf("binary placeholder missing:\n%s", diff)
	}
	if strings.Contains(diff, "@@") {
		t.Errorf("binary file should have no hunk:\n%s", diff)
	}
}

func TestSynthesizeAddDiffTruncation(t *testing.T) {
	// gitShowDiffMaxBytes を確実に超える内容（1 行 100 bytes × 4000 行 ≒ 400KB）
	line := strings.Repeat("x", 99)
	content := strings.Repeat(line+"\n", 4000)
	diff, added := synthesizeAddDiff("big.txt", []byte(content))
	if added != 4000 {
		t.Fatalf("added = %d, want 4000", added)
	}
	if !strings.Contains(diff, "(truncated)") {
		t.Errorf("truncation marker missing (len=%d)", len(diff))
	}
	// 上限 + マーカー + 1 行分程度に収まっていること（無制限に膨らんでいない）
	if len(diff) > gitShowDiffMaxBytes+512 {
		t.Errorf("diff too large after truncation: %d bytes", len(diff))
	}
}

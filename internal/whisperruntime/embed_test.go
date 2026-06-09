package whisperruntime

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestEnsureNoPayloadIsNoOp(t *testing.T) {
	dir := t.TempDir()
	// 同梱物の無い os/arch では何も起きず nil。
	if err := Ensure(dir, "linux-amd64"); err != nil {
		t.Fatalf("Ensure(linux-amd64) = %v, want nil", err)
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Fatalf("binDir should stay empty, got %d entries", len(entries))
	}
}

func TestEnsureSkipsPlaceholders(t *testing.T) {
	dir := t.TempDir()
	// windows-amd64 には README.md / .gitkeep の placeholder しか無い（実 DLL 未配置）。
	// Ensure はそれらをコピーせず、bin/ に DLL を作らない。
	if err := Ensure(dir, "windows-amd64"); err != nil {
		t.Fatalf("Ensure(windows-amd64) = %v, want nil", err)
	}
	entries, _ := os.ReadDir(dir)
	for _, e := range entries {
		// ドットファイル（.gitkeep / .gitignore）と *.md は配置されてはならない。
		if strings.HasPrefix(e.Name(), ".") || filepath.Ext(e.Name()) == ".md" {
			t.Fatalf("placeholder %q must not be laid down", e.Name())
		}
	}
}

func TestEnsureIdempotent(t *testing.T) {
	dir := t.TempDir()
	for i := 0; i < 2; i++ {
		if err := Ensure(dir, "windows-amd64"); err != nil {
			t.Fatalf("Ensure call %d = %v", i, err)
		}
	}
}

func TestHasPayloadFalseForPlaceholderOnly(t *testing.T) {
	// 実 DLL 未配置の間は false。CI/リリースで実 DLL を入れると true になる。
	if HasPayload("windows-amd64") {
		t.Skip("real runtime DLLs are bundled; HasPayload true is expected")
	}
	if HasPayload("linux-amd64") {
		t.Fatalf("HasPayload(linux-amd64) = true, want false")
	}
}

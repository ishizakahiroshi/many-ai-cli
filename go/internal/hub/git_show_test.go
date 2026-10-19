package hub

import (
	"testing"
)

func TestParseNumstatRaw_Basic(t *testing.T) {
	raw := "120\t0\tdocs/foo.md\n15\t8\tsrc/bar.go\n"
	m := parseNumstatRaw(raw)
	if len(m) != 2 {
		t.Fatalf("len = %d, want 2", len(m))
	}
	if m["docs/foo.md"].added != 120 || m["docs/foo.md"].removed != 0 {
		t.Errorf("docs/foo.md: %+v", m["docs/foo.md"])
	}
	if m["src/bar.go"].added != 15 || m["src/bar.go"].removed != 8 {
		t.Errorf("src/bar.go: %+v", m["src/bar.go"])
	}
}

func TestParseNumstatRaw_Binary(t *testing.T) {
	// バイナリファイルは "-\t-\t..." 形式; strconv.Atoi("-") は 0 を返す
	raw := "-\t-\tbinary.png\n"
	m := parseNumstatRaw(raw)
	if e, ok := m["binary.png"]; !ok {
		t.Fatal("binary.png not found")
	} else if e.added != 0 || e.removed != 0 {
		t.Errorf("binary.png: %+v", e)
	}
}

func TestParseNumstatRaw_RenameNewPath(t *testing.T) {
	// rename: "0\t0\told.go => new.go"
	raw := "5\t3\told.go => new.go\n"
	m := parseNumstatRaw(raw)
	if _, ok := m["old.go"]; ok {
		t.Error("old.go should not be a key (rename new path should be used)")
	}
	if e, ok := m["new.go"]; !ok {
		t.Fatal("new.go not found")
	} else if e.added != 5 {
		t.Errorf("new.go added = %d, want 5", e.added)
	}
}

func TestParseNumstatRaw_BraceRenameNewPath(t *testing.T) {
	// "{src => dst}/file.go" 形式（クロスディレクトリ brace 圧縮）。
	// brace 展開で new path "dst/file.go" に復元される（`}` 残存しない）。
	raw := "2\t1\t{src => dst}/file.go\n"
	m := parseNumstatRaw(raw)
	if _, ok := m["dst}/file.go"]; ok {
		t.Error("broken key dst}/file.go must not exist after brace expansion")
	}
	e, ok := m["dst/file.go"]
	if !ok {
		t.Fatalf("dst/file.go not found; keys=%v", mapKeys(m))
	}
	if e.added != 2 || e.removed != 1 {
		t.Errorf("dst/file.go: %+v", e)
	}
}

func TestParseNumstatRaw_Empty(t *testing.T) {
	m := parseNumstatRaw("")
	if len(m) != 0 {
		t.Fatalf("want empty map, got %d entries", len(m))
	}
}

func TestParseNumstatRaw_CRLFLine(t *testing.T) {
	raw := "10\t2\tfile.go\r\n"
	m := parseNumstatRaw(raw)
	if _, ok := m["file.go"]; !ok {
		t.Error("file.go not found (CRLF stripping failed)")
	}
}

// TestApplyNumstat は git_show.go の applyNumstat が parseNumstatRaw を
// 正しく使えていることを確認する（統合テスト）。
func TestApplyNumstat_Integration(t *testing.T) {
	files := []gitShowFile{
		{Path: "src/main.go"},
		{Path: "docs/README.md"},
	}
	raw := "10\t5\tsrc/main.go\n20\t0\tdocs/README.md\n"
	applyNumstat(files, raw)
	if files[0].Added != 10 || files[0].Removed != 5 {
		t.Errorf("main.go: added=%d removed=%d", files[0].Added, files[0].Removed)
	}
	if files[1].Added != 20 || files[1].Removed != 0 {
		t.Errorf("README.md: added=%d removed=%d", files[1].Added, files[1].Removed)
	}
}

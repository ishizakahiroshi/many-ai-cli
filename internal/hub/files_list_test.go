package hub

import (
	"os"
	"path/filepath"
	"testing"
)

func TestWalkFilesLocalIncludesEmptyDirectories(t *testing.T) {
	tmp := t.TempDir()
	emptyDir := filepath.Join(tmp, "docs", "local", "reference")
	if err := os.MkdirAll(emptyDir, 0o755); err != nil {
		t.Fatal(err)
	}

	items, truncated := walkFilesLocal(tmp, tmp)
	if truncated {
		t.Fatal("walkFilesLocal should not truncate small trees")
	}

	for _, item := range items {
		if item.Type == "dir" && item.Path == emptyDir && item.Name == "reference" {
			return
		}
	}
	t.Fatalf("empty directory %q was not returned: %+v", emptyDir, items)
}

func TestWalkFilesLocalIncludesDirectoryOnceWithChildren(t *testing.T) {
	tmp := t.TempDir()
	dir := filepath.Join(tmp, "docs", "local", "reference")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "note.md"), []byte("# note\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	items, _ := walkFilesLocal(tmp, tmp)
	dirCount := 0
	fileCount := 0
	for _, item := range items {
		switch {
		case item.Type == "dir" && item.Path == dir:
			dirCount++
		case item.Type == "file" && item.Name == "note.md":
			fileCount++
		}
	}
	if dirCount != 1 {
		t.Fatalf("dir count = %d, want 1; items=%+v", dirCount, items)
	}
	if fileCount != 1 {
		t.Fatalf("file count = %d, want 1; items=%+v", fileCount, items)
	}
}

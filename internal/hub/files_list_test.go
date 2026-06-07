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

func TestWalkFilesLocalIncludesDotFilesAndDotDirs(t *testing.T) {
	tmp := t.TempDir()
	dotDir := filepath.Join(tmp, ".github")
	if err := os.MkdirAll(dotDir, 0o755); err != nil {
		t.Fatal(err)
	}
	dotFile := filepath.Join(tmp, ".gitignore")
	if err := os.WriteFile(dotFile, []byte("dist/\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	dotDirFile := filepath.Join(dotDir, "workflow.yml")
	if err := os.WriteFile(dotDirFile, []byte("name: ci\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	items, truncated := walkFilesLocal(tmp, tmp)
	if truncated {
		t.Fatal("walkFilesLocal should not truncate small trees")
	}

	if !hasFilesListItem(items, "file", dotFile) {
		t.Fatalf("dotfile %q was not returned: %+v", dotFile, items)
	}
	if !hasFilesListItem(items, "dir", dotDir) {
		t.Fatalf("dotdir %q was not returned: %+v", dotDir, items)
	}
	if !hasFilesListItem(items, "file", dotDirFile) {
		t.Fatalf("dotdir child %q was not returned: %+v", dotDirFile, items)
	}
}

func TestWalkFilesLocalListsGitDirWithoutDescending(t *testing.T) {
	tmp := t.TempDir()
	gitDir := filepath.Join(tmp, ".git")
	if err := os.MkdirAll(filepath.Join(gitDir, "objects"), 0o755); err != nil {
		t.Fatal(err)
	}
	hiddenGitFile := filepath.Join(gitDir, "objects", "pack")
	if err := os.WriteFile(hiddenGitFile, []byte("pack"), 0o644); err != nil {
		t.Fatal(err)
	}

	items, truncated := walkFilesLocal(tmp, tmp)
	if truncated {
		t.Fatal("walkFilesLocal should not truncate small trees")
	}

	if !hasFilesListItem(items, "dir", gitDir) {
		t.Fatalf(".git directory %q was not returned: %+v", gitDir, items)
	}
	if hasFilesListItem(items, "file", hiddenGitFile) {
		t.Fatalf(".git child %q should not be returned: %+v", hiddenGitFile, items)
	}
}

func hasFilesListItem(items []filesListItem, typ, path string) bool {
	for _, item := range items {
		if item.Type == typ && item.Path == path {
			return true
		}
	}
	return false
}

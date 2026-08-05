package hub

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
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

// TestWalkFilesLocalOmitsSummaryForSecretFiles は、秘密情報 denylist 該当ファイルが
// ツリーには出るが summary（本文冒頭）を持たないことを確認する。
// ツリー走査だけで配下の資格情報の中身が一括で吸い出されるのを防ぐため。
func TestWalkFilesLocalOmitsSummaryForSecretFiles(t *testing.T) {
	tmp := t.TempDir()
	if err := os.WriteFile(filepath.Join(tmp, "note.md"), []byte("# note\n\nbody text\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(tmp, ".credentials.json"), []byte(`{"accessToken":"super-secret"}`), 0o600); err != nil { // secrets-scan: allow
		t.Fatal(err)
	}

	items, _ := walkFilesLocal(tmp, tmp)
	var sawNote, sawCred bool
	for _, item := range items {
		switch item.Name {
		case "note.md":
			sawNote = true
			if item.Summary == "" {
				t.Fatal("note.md should still have a summary")
			}
		case ".credentials.json": // secrets-scan: allow
			sawCred = true
			if item.Summary != "" {
				t.Fatalf(".credentials.json summary should be empty, got %q", item.Summary) // secrets-scan: allow
			}
		}
	}
	if !sawNote || !sawCred {
		t.Fatalf("expected both files in listing: note=%v cred=%v", sawNote, sawCred)
	}
}

// TestHandleFilesList_ArbitraryRootScopeByCallerOrigin は、cwd/git root の外を ?root= に
// 指定したときの扱いが呼び出し元で変わることを確認する。直 loopback は列挙でき、
// 論理リモートは従来どおり 403。方針の根拠は internal/hub/files_scope.go。
func TestHandleFilesList_ArbitraryRootScopeByCallerOrigin(t *testing.T) {
	setSecTestHome(t)
	projDir := t.TempDir()
	outsideDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(outsideDir, "recap_outside.md"), []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := newTestFilesContentServer(projDir)

	call := func(remoteAddr string) (int, string) {
		req := newFilesReq(t, "/api/files-list?token=tok&root="+url.QueryEscape(outsideDir), remoteAddr)
		w := httptest.NewRecorder()
		s.handleFilesList(w, req)
		return w.Code, w.Body.String()
	}

	code, body := call(testRemoteAddrLoopback)
	if code != http.StatusOK {
		t.Fatalf("loopback: expected 200, got %d: %s", code, body)
	}
	if !strings.Contains(body, "recap_outside.md") {
		t.Fatalf("loopback: expected outside file in listing: %s", body)
	}

	code, body = call(testRemoteAddrRemote)
	if code != http.StatusForbidden {
		t.Fatalf("remote: expected 403, got %d: %s", code, body)
	}
	if !strings.Contains(body, "outside allowed roots") {
		t.Fatalf("remote: unexpected 403 reason: %s", body)
	}
}

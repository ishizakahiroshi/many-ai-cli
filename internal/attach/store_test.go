package attach

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"
)

func TestSave(t *testing.T) {
	tests := []struct {
		provider   string
		wantPrefix string
		wantSuffix string
	}{
		{provider: "claude", wantPrefix: "@", wantSuffix: " "},
		{provider: "codex", wantPrefix: "@", wantSuffix: " "},
	}

	for _, tc := range tests {
		tc := tc
		t.Run(tc.provider, func(t *testing.T) {
			baseDir := t.TempDir()
			data := []byte("fake-png-bytes")

			path, inject, err := Save(baseDir, 42, tc.provider, data, "")
			if err != nil {
				t.Fatalf("Save returned error: %v", err)
			}

			// file must exist
			if _, statErr := os.Stat(path); statErr != nil {
				t.Fatalf("saved file not found at %s: %v", path, statErr)
			}

			// file must be under baseDir/42/
			expectedDir := filepath.Join(baseDir, "42")
			if !strings.HasPrefix(path, expectedDir) {
				t.Errorf("path %q does not start with %q", path, expectedDir)
			}

			// inject prefix/suffix
			if tc.wantPrefix != "" && !strings.HasPrefix(inject, tc.wantPrefix) {
				t.Errorf("inject %q: want prefix %q", inject, tc.wantPrefix)
			}
			if !strings.HasSuffix(inject, tc.wantSuffix) {
				t.Errorf("inject %q: want suffix %q", inject, tc.wantSuffix)
			}

			// inject must contain the absolute path (normalize separators for cross-platform comparison)
			if !strings.Contains(filepath.ToSlash(inject), filepath.ToSlash(path)) {
				t.Errorf("inject %q does not contain path %q", inject, path)
			}
		})
	}
}

func TestSaveDoesNotOverwriteRepeatedAttachments(t *testing.T) {
	baseDir := t.TempDir()

	firstPath, _, err := Save(baseDir, 42, "codex", []byte("first"), "note.txt")
	if err != nil {
		t.Fatalf("Save first: %v", err)
	}
	secondPath, _, err := Save(baseDir, 42, "codex", []byte("second"), "note.txt")
	if err != nil {
		t.Fatalf("Save second: %v", err)
	}

	if firstPath == secondPath {
		t.Fatalf("repeated attachments used the same path: %s", firstPath)
	}

	firstData, err := os.ReadFile(firstPath)
	if err != nil {
		t.Fatalf("ReadFile first: %v", err)
	}
	secondData, err := os.ReadFile(secondPath)
	if err != nil {
		t.Fatalf("ReadFile second: %v", err)
	}
	if string(firstData) != "first" {
		t.Fatalf("first attachment was overwritten: got %q", string(firstData))
	}
	if string(secondData) != "second" {
		t.Fatalf("second attachment content mismatch: got %q", string(secondData))
	}
}

func TestSaveUsesTimestampWithNanosecondsInFilename(t *testing.T) {
	baseDir := t.TempDir()

	// .png は allowlist に含まれるので拡張子がそのまま使われる
	path, _, err := Save(baseDir, 42, "codex", []byte("data"), "note.png")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}

	name := filepath.Base(path)
	// 拡張子は allowlist に含まれるもの（.png 等）か magic-byte 判定結果
	if !regexp.MustCompile(`^\d{14}_\d{9}\.[a-z]+$`).MatchString(name) {
		t.Fatalf("saved filename %q does not match timestamp_nanoseconds format", name)
	}
}

func TestCleanOld(t *testing.T) {
	baseDir := t.TempDir()

	// create a fresh file via Save
	path, _, err := Save(baseDir, 1, "claude", []byte("data"), "")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}

	// backdate mtime to 8 days ago so it falls outside a 7-day retention window
	old := time.Now().Add(-8 * 24 * time.Hour)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatalf("Chtimes: %v", err)
	}

	// create a recent file that must NOT be deleted
	recentPath, _, err := Save(baseDir, 2, "codex", []byte("recent"), "")
	if err != nil {
		t.Fatalf("Save recent: %v", err)
	}

	if err := CleanOld(baseDir, 7); err != nil {
		t.Fatalf("CleanOld: %v", err)
	}

	// old file must be gone
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Errorf("expected old file %s to be deleted", path)
	}

	// recent file must still exist
	if _, err := os.Stat(recentPath); err != nil {
		t.Errorf("recent file %s should still exist: %v", recentPath, err)
	}
}

// TestCleanOldRemovesEmptySessionDirs は古いファイルが削除されて空になったサブディレクトリが
// CleanOld によって削除されることを確認する。
func TestCleanOldRemovesEmptySessionDirs(t *testing.T) {
	baseDir := t.TempDir()

	// セッション 99 のファイルを作成して古い mtime に書き換え
	path, _, err := Save(baseDir, 99, "claude", []byte("old"), "")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	old := time.Now().Add(-8 * 24 * time.Hour)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatalf("Chtimes: %v", err)
	}

	if err := CleanOld(baseDir, 7); err != nil {
		t.Fatalf("CleanOld: %v", err)
	}

	// ファイルが消えたセッションディレクトリも消えているはず
	sessionDir := filepath.Join(baseDir, "99")
	if _, err := os.Stat(sessionDir); !os.IsNotExist(err) {
		t.Errorf("expected empty session dir %s to be removed", sessionDir)
	}
}

func TestSaveKeepsOriginalExtension(t *testing.T) {
	baseDir := t.TempDir()

	pngData := []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}
	savedPath, _, err := Save(baseDir, 1, "claude", pngData, "evil.exe")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	ext := strings.ToLower(filepath.Ext(savedPath))
	if ext != ".exe" {
		t.Fatalf("expected original .exe extension, got %q", ext)
	}
}

func TestSaveDetectsExtensionWhenFilenameHasNoExtension(t *testing.T) {
	baseDir := t.TempDir()

	pngData := []byte{0x89, 'P', 'N', 'G', 0x0d, 0x0a, 0x1a, 0x0a}
	savedPath, _, err := Save(baseDir, 1, "claude", pngData, "clipboard-image")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	ext := strings.ToLower(filepath.Ext(savedPath))
	if ext != ".png" {
		t.Fatalf("expected .png from magic-byte detection, got %q", ext)
	}
}

func TestSaveKeepsCSVExtension(t *testing.T) {
	baseDir := t.TempDir()

	savedPath, _, err := Save(baseDir, 1, "codex", []byte("name,value\nalice,1\n"), "report.csv")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	ext := strings.ToLower(filepath.Ext(savedPath))
	if ext != ".csv" {
		t.Fatalf("expected .csv, got %q", ext)
	}
}

func TestSaveKeepsSourceFileExtensions(t *testing.T) {
	baseDir := t.TempDir()
	for _, name := range []string{"index.php", "app.js", "main.go", "page.html", "component.vue", "script.ts"} {
		name := name
		t.Run(name, func(t *testing.T) {
			savedPath, _, err := Save(baseDir, 1, "codex", []byte("source"), name)
			if err != nil {
				t.Fatalf("Save: %v", err)
			}
			want := strings.ToLower(filepath.Ext(name))
			got := strings.ToLower(filepath.Ext(savedPath))
			if got != want {
				t.Fatalf("expected %s, got %q", want, got)
			}
		})
	}
}

func TestSaveKeepsOfficeDocumentExtensions(t *testing.T) {
	baseDir := t.TempDir()
	for _, name := range []string{"book.xls", "book.xlsx", "report.doc", "report.docx", "deck.ppt", "deck.pptx", "paper.pdf"} {
		name := name
		t.Run(name, func(t *testing.T) {
			savedPath, _, err := Save(baseDir, 1, "codex", []byte("document"), name)
			if err != nil {
				t.Fatalf("Save: %v", err)
			}
			want := strings.ToLower(filepath.Ext(name))
			got := strings.ToLower(filepath.Ext(savedPath))
			if got != want {
				t.Fatalf("expected %s, got %q", want, got)
			}
		})
	}
}

func TestSaveUnknownExtensionFallsBackToBin(t *testing.T) {
	baseDir := t.TempDir()

	savedPath, _, err := Save(baseDir, 1, "codex", []byte("plain text without known extension"), "")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	ext := strings.ToLower(filepath.Ext(savedPath))
	if ext != ".bin" {
		t.Fatalf("expected .bin fallback, got %q", ext)
	}
}

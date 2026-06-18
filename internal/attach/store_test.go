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

	path, _, err := Save(baseDir, 42, "codex", []byte("data"), "売上 report?.PNG")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}

	name := filepath.Base(path)
	if !regexp.MustCompile(`^\d{14}_\d{9}_売上_report\.png$`).MatchString(name) {
		t.Fatalf("saved filename %q does not retain sanitized original name", name)
	}
}

func TestSanitizeFilename(t *testing.T) {
	tests := []struct {
		name     string
		filename string
		ext      string
		want     string
	}{
		{name: "japanese", filename: "月次 売上.csv", ext: ".csv", want: "月次_売上.csv"},
		{name: "path traversal", filename: `..\secret/report?.CSV`, ext: ".csv", want: "report.csv"},
		{name: "windows invalid characters", filename: `a<b>:c"d|e?f*.txt`, ext: ".txt", want: "a_b_c_d_e_f.txt"},
		{name: "empty", filename: `../...`, ext: ".bin", want: "attachment.bin"},
		{name: "repeated separators", filename: "a \t ? b.csv", ext: ".csv", want: "a_b.csv"},
		{name: "cli punctuation", filename: `a&b;c#d@e(f)[g]{h}!$'i.png`, ext: ".png", want: "a_b_c_d_e_f_g_h_i.png"},
		{name: "unicode letters and marks", filename: "cafe\u0301_日本語.pdf", ext: ".pdf", want: "cafe\u0301_日本語.pdf"},
		{name: "emoji", filename: "diagram📎final.png", ext: ".png", want: "diagram_final.png"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := sanitizeFilename(tc.filename, tc.ext); got != tc.want {
				t.Fatalf("sanitizeFilename(%q, %q) = %q, want %q", tc.filename, tc.ext, got, tc.want)
			}
		})
	}
}

func TestSanitizeFilenameLimitsUTF8ByteLength(t *testing.T) {
	got := sanitizeFilename(strings.Repeat("長", maxSanitizedFilenameBytes)+".csv", ".csv")
	if len(got) > maxSanitizedFilenameBytes {
		t.Fatalf("sanitized filename byte length = %d, want <= %d", len(got), maxSanitizedFilenameBytes)
	}
	if !strings.HasSuffix(got, ".csv") {
		t.Fatalf("sanitized filename %q lost extension", got)
	}
}

func TestSavePreservesSupportedAttachmentTypes(t *testing.T) {
	tests := []struct {
		name     string
		filename string
		data     []byte
		wantExt  string
	}{
		{name: "csv filename", filename: "売上.CSV", data: []byte("a,b\n1,2\n"), wantExt: ".csv"},
		{name: "png magic", data: []byte{0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A}, wantExt: ".png"},
		{name: "jpeg magic", data: []byte{0xFF, 0xD8, 0xFF, 0xE0}, wantExt: ".jpg"},
		{name: "gif magic", data: []byte("GIF89a"), wantExt: ".gif"},
		{name: "webp magic", data: []byte("RIFFxxxxWEBP"), wantExt: ".webp"},
		{name: "pdf magic", data: []byte("%PDF-1.7"), wantExt: ".pdf"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			path, _, err := Save(t.TempDir(), 1, "codex", tc.data, tc.filename)
			if err != nil {
				t.Fatalf("Save: %v", err)
			}
			if got := filepath.Ext(path); got != tc.wantExt {
				t.Fatalf("saved extension = %q, want %q (path %q)", got, tc.wantExt, path)
			}
			if tc.filename == "" && !strings.Contains(filepath.Base(path), "_attachment"+tc.wantExt) {
				t.Fatalf("unnamed attachment path %q does not use fallback name", path)
			}
		})
	}
}

func TestSavePrefixPreventsWindowsReservedBasename(t *testing.T) {
	path, _, err := Save(t.TempDir(), 1, "codex", []byte("data"), "CON.txt")
	if err != nil {
		t.Fatalf("Save: %v", err)
	}
	name := filepath.Base(path)
	if strings.EqualFold(name, "CON.txt") || !strings.HasSuffix(name, "_CON.txt") {
		t.Fatalf("saved filename %q does not safely prefix reserved basename", name)
	}
}

func TestAttachmentInject(t *testing.T) {
	windowsLikePath := `C:\Users\John Smith\開発 (試験)\file.png`
	tests := []struct {
		provider string
		path     string
		want     string
	}{
		{provider: "claude", path: windowsLikePath, want: "@" + windowsLikePath + " "},
		{provider: "codex", path: windowsLikePath, want: "@C:/Users/John Smith/開発 (試験)/file.png "},
		{provider: "other", path: windowsLikePath, want: windowsLikePath + " "},
	}

	for _, tc := range tests {
		t.Run(tc.provider, func(t *testing.T) {
			got, err := attachmentInject(tc.provider, tc.path)
			if err != nil {
				t.Fatalf("attachmentInject: %v", err)
			}
			if got != tc.want {
				t.Fatalf("attachmentInject(%q, %q) = %q, want %q", tc.provider, tc.path, got, tc.want)
			}
		})
	}
}

func TestAttachmentInjectRejectsControlCharacters(t *testing.T) {
	if _, err := attachmentInject("codex", "C:/tmp/bad\nname.png"); err == nil {
		t.Fatal("attachmentInject accepted a path containing a newline")
	}
}

func TestSaveRemovesFileWhenInjectPathIsUnsafe(t *testing.T) {
	baseDir := filepath.Join(t.TempDir(), "bad\u200bparent")
	path, inject, err := Save(baseDir, 1, "codex", []byte("data"), "note.txt")
	if err == nil {
		t.Fatal("Save accepted an unsafe attachment path")
	}
	if path != "" || inject != "" {
		t.Fatalf("Save returned path=%q inject=%q on error", path, inject)
	}
	sessionDir := filepath.Join(baseDir, "1")
	entries, readErr := os.ReadDir(sessionDir)
	if readErr != nil && !os.IsNotExist(readErr) {
		t.Fatalf("ReadDir: %v", readErr)
	}
	if len(entries) != 0 {
		t.Fatalf("unsafe attachment left %d file(s) behind", len(entries))
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

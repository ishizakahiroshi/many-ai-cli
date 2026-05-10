package attach

import (
	"os"
	"path/filepath"
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
		{provider: "claude", wantPrefix: "@", wantSuffix: "\r"},
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

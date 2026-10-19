package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestCleanOrchestrationArtifactsAppliesAgeAndCountButKeepsActive(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	base, err := orchestrationDir()
	if err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 9, 3, 1, 0, 0, 0, time.UTC)
	makeDir := func(name string, modTime time.Time) string {
		t.Helper()
		dir := filepath.Join(base, name)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "board.md"), []byte("board\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		if err := os.Chtimes(dir, modTime, modTime); err != nil {
			t.Fatal(err)
		}
		return dir
	}
	for i := 0; i < 205; i++ {
		makeDir(fmt.Sprintf("recent-%03d", i), now.Add(-time.Duration(i)*time.Hour))
	}
	expired := makeDir("expired", now.Add(-31*24*time.Hour))
	active := makeDir("active", now.Add(-60*24*time.Hour))

	s := newTestServer()
	ses := registerTestSession(s, 77, "codex")
	ses.OrchestrationID = "active"
	ses.BoardPath = filepath.Join(active, "board.md")
	if err := s.cleanOrchestrationArtifactsAt(now); err != nil {
		t.Fatal(err)
	}
	entries, err := os.ReadDir(base)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != orchestrationMaxDirs {
		t.Fatalf("orchestration directories = %d, want %d", len(entries), orchestrationMaxDirs)
	}
	if _, err := os.Stat(active); err != nil {
		t.Fatalf("active orchestration removed: %v", err)
	}
	if _, err := os.Stat(expired); !os.IsNotExist(err) {
		t.Fatalf("expired orchestration retained: %v", err)
	}
	if _, err := os.Stat(filepath.Join(base, "recent-000")); err != nil {
		t.Fatalf("newest orchestration removed: %v", err)
	}
}

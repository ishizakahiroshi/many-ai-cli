package hub

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestUsageProbeManagerRejectsDuplicateAndCancels(t *testing.T) {
	manager := newUsageProbeManager()
	ctx, cancel := context.WithCancel(context.Background())
	state := &usageProbeState{cancel: cancel}
	key := usageProbeKey("claude", "max-a")
	if !manager.begin(key, state) {
		t.Fatal("first usage probe was not registered")
	}
	if manager.begin(key, &usageProbeState{}) {
		t.Fatal("duplicate usage probe was accepted")
	}
	if !manager.cancel(key) {
		t.Fatal("running usage probe was not cancellable")
	}
	if ctx.Err() != context.Canceled {
		t.Fatalf("cancel did not reach probe context: %v", ctx.Err())
	}
	manager.finish(key)
	if manager.isRunning("claude", "max-a") {
		t.Fatal("finished usage probe still reported as running")
	}
}

func TestCleanupUsageProbeTranscriptsKeepsNewestAndProtectsOtherProjects(t *testing.T) {
	root := t.TempDir()
	profileDir := filepath.Join(root, "profile")
	probeCWD := filepath.Join(root, "usage-probe")
	target := filepath.Join(profileDir, "projects", claudeProjectDirName(probeCWD))
	other := filepath.Join(profileDir, "projects", claudeProjectDirName(filepath.Join(root, "usage-probe-extra")))
	if err := os.MkdirAll(target, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(other, 0o700); err != nil {
		t.Fatal(err)
	}
	base := time.Now().Add(-10 * time.Minute)
	for i := 0; i < 5; i++ {
		path := filepath.Join(target, "probe-"+string(rune('a'+i))+".jsonl")
		if err := os.WriteFile(path, []byte("synthetic"), 0o600); err != nil {
			t.Fatal(err)
		}
		stamp := base.Add(time.Duration(i) * time.Minute)
		if err := os.Chtimes(path, stamp, stamp); err != nil {
			t.Fatal(err)
		}
	}
	otherPath := filepath.Join(other, "user-transcript.jsonl")
	if err := os.WriteFile(otherPath, []byte("must remain"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "keep.txt"), []byte("not a transcript"), 0o600); err != nil {
		t.Fatal(err)
	}

	removed, err := cleanupUsageProbeTranscripts(profileDir, probeCWD)
	if err != nil {
		t.Fatal(err)
	}
	if removed != 2 {
		t.Fatalf("removed=%d, want 2", removed)
	}
	entries, err := os.ReadDir(target)
	if err != nil {
		t.Fatal(err)
	}
	jsonlCount := 0
	for _, entry := range entries {
		if filepath.Ext(entry.Name()) == ".jsonl" {
			jsonlCount++
		}
	}
	if jsonlCount != usageProbeTranscriptKeep {
		t.Fatalf("probe transcript count=%d, want %d", jsonlCount, usageProbeTranscriptKeep)
	}
	if _, err := os.Stat(otherPath); err != nil {
		t.Fatalf("other project transcript was removed: %v", err)
	}
	if _, err := os.Stat(filepath.Join(target, "keep.txt")); err != nil {
		t.Fatalf("non-transcript file was removed: %v", err)
	}
}

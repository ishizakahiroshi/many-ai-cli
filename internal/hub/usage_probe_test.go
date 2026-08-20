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

func TestUsageProbeConfirmDialogMatchesStartupPickers(t *testing.T) {
	cases := []struct {
		name   string
		screen string
		want   bool
	}{
		{
			name: "folder trust",
			screen: "Quick safety check\n" +
				"Is this a project you created or one you trust?\n" +
				"1. Yes, I trust this folder\n" +
				"2. No, exit\n" +
				"Enter to confirm",
			want: true,
		},
		{
			name: "external imports",
			screen: "Allow external CLAUDE.md file imports?\n" +
				"This project's CLAUDE.md imports files outside the current working directory.\n" +
				"1. Yes, allow external imports\n" +
				"2. No, disable external imports",
			want: true,
		},
		{
			name:   "ready prompt",
			screen: "Try something like:\n> write a test\n",
			want:   false,
		},
		{
			name: "tool approval",
			screen: "Bash command\nls\nDo you want to proceed?\n" +
				"1. Yes\n2. No\nEnter to confirm",
			want: false,
		},
		{name: "empty", screen: "", want: false},
	}
	for _, tc := range cases {
		if got := usageProbeConfirmDialog(tc.screen); got != tc.want {
			t.Fatalf("%s: got %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestUsageProbeScreenIsConfirmDialogReadsVT(t *testing.T) {
	s := &Server{sessions: map[int]*session{}}
	vt := newVTBuffer(80, 12)
	vt.Write([]byte("Is this a project you created or one you trust?\r\nYes, I trust this folder\r\n"))
	s.sessions[3] = &session{ID: 3, vt: vt}
	if !s.usageProbeScreenIsConfirmDialog(3) {
		t.Fatal("folder-trust screen was not detected")
	}
	if s.usageProbeScreenIsConfirmDialog(99) {
		t.Fatal("missing session looked like a confirm dialog")
	}
}

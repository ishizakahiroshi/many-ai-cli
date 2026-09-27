package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/provider"
)

func newTestRecoverStore(t *testing.T) (*provider.HistoryStore, string) {
	t.Helper()
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := provider.NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	return store, root
}

func breakHead(t *testing.T, root, providerID string) {
	t.Helper()
	path := filepath.Join(root, providerID, "HEAD")
	if err := os.WriteFile(path, []byte("missing-revision\n"), 0o600); err != nil {
		t.Fatal(err)
	}
}

func testRecoverDefinition(id, name string) provider.Definition {
	return provider.Definition{SchemaVersion: 1, ID: id, DisplayName: name, Launch: &provider.LaunchDefinition{Executable: id}}
}

func TestProviderCommandRecoverRequiresProviderID(t *testing.T) {
	store, _ := newTestRecoverStore(t)
	var buf bytes.Buffer
	err := runProviderRecoverCommand(store, nil, &buf)
	if err == nil {
		t.Fatal("expected error for missing provider id")
	}
	if err.Error() != "provider recover <provider-id> [revision|--list]" {
		t.Fatalf("unexpected error message: %v", err)
	}
	if buf.Len() != 0 {
		t.Fatalf("expected no output, got %q", buf.String())
	}
}

func TestProviderCommandRecoverListShowsCandidateAndBase(t *testing.T) {
	store, root := newTestRecoverStore(t)
	saved, err := store.SaveOverride("claude", testRecoverDefinition("claude", "Claude"), provider.Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	breakHead(t, root, "claude")

	headPath := filepath.Join(root, "claude", "HEAD")
	before, err := os.ReadFile(headPath)
	if err != nil {
		t.Fatal(err)
	}

	var buf bytes.Buffer
	if err := runProviderRecoverCommand(store, []string{"claude", "--list"}, &buf); err != nil {
		t.Fatal(err)
	}

	after, err := os.ReadFile(headPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) {
		t.Fatalf("--list must not write: HEAD before=%q after=%q", before, after)
	}

	out := buf.String()
	lines := strings.Split(strings.TrimRight(out, "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 candidate lines, got %d: %q", len(lines), out)
	}
	wantFirst := "candidate\tuser_revision\t" + saved.Revision + "\t" + saved.CreatedAt
	if lines[0] != wantFirst {
		t.Fatalf("line 1 = %q, want %q", lines[0], wantFirst)
	}
	if lines[1] != "candidate\tbase" {
		t.Fatalf("line 2 = %q, want %q", lines[1], "candidate\tbase")
	}
}

func TestProviderCommandRecoverListWithoutOverrideOnlyShowsBase(t *testing.T) {
	store, _ := newTestRecoverStore(t)
	var buf bytes.Buffer
	if err := runProviderRecoverCommand(store, []string{"claude", "--list"}, &buf); err != nil {
		t.Fatal(err)
	}
	if got := buf.String(); got != "candidate\tbase\n" {
		t.Fatalf("output = %q, want %q", got, "candidate\tbase\n")
	}
}

func TestProviderCommandRecoverToChosenRevision(t *testing.T) {
	store, root := newTestRecoverStore(t)
	saved, err := store.SaveOverride("claude", testRecoverDefinition("claude", "Claude"), provider.Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	breakHead(t, root, "claude")

	var buf bytes.Buffer
	if err := runProviderRecoverCommand(store, []string{"claude", saved.Revision}, &buf); err != nil {
		t.Fatal(err)
	}
	out := strings.TrimSpace(buf.String())
	if !strings.HasPrefix(out, "recovered\t") {
		t.Fatalf("output = %q, want prefix %q", out, "recovered\t")
	}
	newRevision := strings.TrimPrefix(out, "recovered\t")
	if newRevision == "" || newRevision == saved.Revision {
		t.Fatalf("recovered revision = %q, want a new non-empty revision", newRevision)
	}

	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != newRevision || current.Payload.DisplayName != "Claude" {
		t.Fatalf("current after recover = %#v", current)
	}
}

func TestProviderCommandRecoverToBaseWithoutRevision(t *testing.T) {
	store, root := newTestRecoverStore(t)
	if _, err := store.SaveOverride("claude", testRecoverDefinition("claude", "Claude"), provider.Definition{}, "", "edit"); err != nil {
		t.Fatal(err)
	}
	breakHead(t, root, "claude")

	var buf bytes.Buffer
	if err := runProviderRecoverCommand(store, []string{"claude"}, &buf); err != nil {
		t.Fatal(err)
	}
	out := strings.TrimSpace(buf.String())
	if !strings.HasPrefix(out, "recovered\t") {
		t.Fatalf("output = %q, want prefix %q", out, "recovered\t")
	}

	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Payload.DisplayName != "" {
		t.Fatalf("current payload after base recover = %#v, want override-free", current.Payload)
	}
}

func TestProviderCommandRecoverRejectsHealthyHead(t *testing.T) {
	store, _ := newTestRecoverStore(t)
	if _, err := store.SaveOverride("claude", testRecoverDefinition("claude", "Claude"), provider.Definition{}, "", "edit"); err != nil {
		t.Fatal(err)
	}

	var buf bytes.Buffer
	if err := runProviderRecoverCommand(store, []string{"claude"}, &buf); err == nil {
		t.Fatal("expected error recovering a healthy HEAD")
	}
	if buf.Len() != 0 {
		t.Fatalf("expected no output on error, got %q", buf.String())
	}
}

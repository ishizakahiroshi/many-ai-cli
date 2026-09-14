package provider

import (
	"os"
	"path/filepath"
	"testing"
)

func TestHistoryStoreCreatesImmutableRevisionsAndBackups(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	second, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Changed", Launch: &LaunchDefinition{Executable: "claude"}}, first.Revision, "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(backups, "claude", first.Revision+".json")); err != nil {
		t.Fatalf("first revision backup missing: %v", err)
	}
	if first.Revision == second.Revision {
		t.Fatal("successive revisions reused the same id")
	}
	restored, err := store.Restore("claude", first.Revision, second.Revision)
	if err != nil {
		t.Fatal(err)
	}
	if restored.Reason != "restore" || restored.Payload.DisplayName != "Claude" {
		t.Fatalf("restored revision = %#v", restored)
	}
	revisions, err := store.List("claude")
	if err != nil {
		t.Fatal(err)
	}
	if len(revisions) != 3 {
		t.Fatalf("revision count = %d, want 3", len(revisions))
	}
}

func TestHistoryStoreRejectsRevisionConflictAndTraversal(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Other", Launch: &LaunchDefinition{Executable: "example"}}, "wrong", "edit"); err == nil {
		t.Fatal("stale expected revision was accepted")
	}
	if _, err := store.Current("../escape"); err == nil {
		t.Fatal("path traversal provider id was accepted")
	}
	if _, err := store.Restore("example", "../escape", first.Revision); err == nil {
		t.Fatal("path traversal revision was accepted")
	}
}

func TestLoadOverridesQuarantinesCorruptHead(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, "", "edit"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "claude", "HEAD"), []byte("missing-revision\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	definitions, diagnostics, err := store.LoadOverrides()
	if err != nil {
		t.Fatal(err)
	}
	if len(definitions) != 0 || len(diagnostics) == 0 {
		t.Fatalf("LoadOverrides = %#v, %#v, want diagnostic and no effective override", definitions, diagnostics)
	}
	entries, err := os.ReadDir(filepath.Join(backups, "quarantine"))
	if err != nil || len(entries) == 0 {
		t.Fatalf("quarantine entries = %#v, %v", entries, err)
	}
}

func TestVerifyBackupRejectsCorruptBackup(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Changed", Launch: &LaunchDefinition{Executable: "claude"}}, first.Revision, "edit"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(backups, "claude", first.Revision+".json"), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.VerifyBackup("claude", first.Revision); err == nil {
		t.Fatal("corrupt backup was accepted")
	}
}

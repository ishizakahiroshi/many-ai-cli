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

// TestHistoryStoreRejectsTraversalProviderIDOnRestoreAndBackup は gosec G703 で
// 見つかった穴を塞いだままにする。Restore() は providerID を検査せずに
// readRevisionLocked を呼んでいた（検査されていたのは revision だけ）。
// backup 側の provider id は **ディスク上のファイルの中身**から来るので、
// 引数の検査とは別に検査が要る。
func TestHistoryStoreRejectsTraversalProviderIDOnRestoreAndBackup(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	saved, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}

	// revision は正しく、providerID だけが root の外を指す。
	if _, err := store.Restore("../escape", saved.Revision, ""); err == nil {
		t.Fatal("Restore accepted a path traversal provider id")
	}

	// 細工された revision ファイルから読み戻したレコードを模した入力。
	if err := store.writeBackupLocked(RevisionRecord{ProviderID: "../escape", Revision: saved.Revision}); err == nil {
		t.Fatal("writeBackupLocked accepted a path traversal provider id from record content")
	}
	if err := store.writeBackupLocked(RevisionRecord{ProviderID: "example", Revision: "../escape"}); err == nil {
		t.Fatal("writeBackupLocked accepted a path traversal revision from record content")
	}

	// RestoreBackup は backupID を検査せず Join していた（VerifyBackup 側にだけ検査があった）。
	if _, err := store.RestoreBackup("example", "../escape", ""); err == nil {
		t.Fatal("RestoreBackup accepted a path traversal backup id")
	}
	if _, err := store.VerifyBackup("example", "../escape"); err == nil {
		t.Fatal("VerifyBackup accepted a path traversal backup id")
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

package provider

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestHistoryStoreCreatesImmutableRevisionsAndBackups(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	second, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Changed", Launch: &LaunchDefinition{Executable: "claude"}}, Definition{}, first.Revision, "edit")
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
	first, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}, Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Other", Launch: &LaunchDefinition{Executable: "example"}}, Definition{}, "wrong", "edit"); err == nil {
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
	saved, err := store.SaveOverride("example", Definition{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}, Definition{}, "", "edit")
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
	if _, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, Definition{}, "", "edit"); err != nil {
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
	if _, err := os.Stat(filepath.Join(root, "claude", "HEAD")); err != nil {
		t.Fatalf("corrupt HEAD was deleted instead of copied: %v", err)
	}
}

func TestVerifyBackupRejectsCorruptBackup(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}, Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("claude", Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Changed", Launch: &LaunchDefinition{Executable: "claude"}}, Definition{}, first.Revision, "edit"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(backups, "claude", first.Revision+".json"), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.VerifyBackup("claude", first.Revision); err == nil {
		t.Fatal("corrupt backup was accepted")
	}
}

func testOverride(id, name string) Definition {
	return Definition{SchemaVersion: 1, ID: id, DisplayName: name, Launch: &LaunchDefinition{Executable: id}}
}

func TestHistoryStoreMissingRevisionRestoreKeepsCurrent(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.Restore("example", "missing-revision", first.Revision); err == nil {
		t.Fatal("missing revision restore was accepted")
	}
	current, err := store.Current("example")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != first.Revision {
		t.Fatalf("current revision = %s, want %s", current.Revision, first.Revision)
	}
}

func TestHistoryStoreCorruptBackupRestoreKeepsCurrent(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	second, err := store.SaveOverride("example", testOverride("example", "Changed"), Definition{}, first.Revision, "edit")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(backups, "example", first.Revision+".json"), []byte("{not-json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.RestoreBackup("example", first.Revision, second.Revision); err == nil {
		t.Fatal("corrupt backup restore was accepted")
	}
	current, err := store.Current("example")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != second.Revision {
		t.Fatalf("current revision = %s, want %s", current.Revision, second.Revision)
	}
}

func TestHistoryStoreOldSchemaRevisionIsRejected(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	store, err := NewHistoryStore(root, filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, "example", "revisions", first.Revision+".json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(`{"schema_version":99,"provider_id":"example","revision":"`+first.Revision+`","content_digest":"deadbeef","payload":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Current("example"); err == nil {
		t.Fatal("old schema revision was accepted")
	}
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	current, err := store.Current("example")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != first.Revision {
		t.Fatalf("restored original revision = %s, want %s", current.Revision, first.Revision)
	}
}

func TestHistoryStoreRejectsSecretEnvValuesAndHugePayload(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	store, err := NewHistoryStore(root, filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	secret := testOverride("example", "Example")
	secret.Launch.AllowedEnv = []string{"TOKEN=not-a-name"}
	if _, err := store.SaveOverride("example", secret, Definition{}, "", "edit"); err == nil {
		t.Fatal("environment value was stored as an override")
	}
	huge := testOverride("example", strings.Repeat("a", MaxStringLength+1))
	if _, err := store.SaveOverride("example", huge, Definition{}, "", "edit"); err == nil {
		t.Fatal("oversized display name was stored as an override")
	}
	if _, err := os.Stat(filepath.Join(root, "example", "HEAD")); !os.IsNotExist(err) {
		t.Fatalf("rejected payload still wrote HEAD: %v", err)
	}
}

func TestHistoryStoreBackupWriteFailureKeepsCurrent(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups-file")
	if err := os.WriteFile(backups, []byte("not-a-directory"), 0o600); err != nil {
		t.Fatal(err)
	}
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("example", testOverride("example", "Changed"), Definition{}, first.Revision, "edit"); err == nil {
		t.Fatal("save succeeded despite unwritable backup root")
	}
	current, err := store.Current("example")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != first.Revision || current.Payload.DisplayName != "Example" {
		t.Fatalf("current after failed save = %#v", current)
	}
}

func TestHistoryStoreLeftoverTempDoesNotHideCurrent(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	store, err := NewHistoryStore(root, filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	tmp := filepath.Join(root, "example", ".provider-atomic-interrupted")
	if err := os.WriteFile(tmp, []byte("{partial"), 0o600); err != nil {
		t.Fatal(err)
	}
	current, err := store.Current("example")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != first.Revision {
		t.Fatalf("current revision = %s, want %s", current.Revision, first.Revision)
	}
}

// TestHistoryStoreRejectsMixedProviderRevision pins a gap the digest check
// alone cannot close: a revision file is internally self-consistent
// (ContentDigest matches its own Payload), so copying provider A's revision
// file verbatim into provider B's directory would pass the digest check
// while returning A's metadata/payload as if it belonged to B. Every reader
// must also confirm the loaded record's ProviderID/Payload.ID match the
// provider directory it was read from.
func TestHistoryStoreRejectsMixedProviderRevision(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	aRevision, err := store.SaveOverride("provider-a", testOverride("provider-a", "A"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	bRevision, err := store.SaveOverride("provider-b", testOverride("provider-b", "B"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}

	// Copy provider-a's self-consistent revision file into provider-b's
	// revisions directory under a name provider-b would plausibly have.
	aPath := filepath.Join(root, "provider-a", "revisions", aRevision.Revision+".json")
	raw, err := os.ReadFile(aPath)
	if err != nil {
		t.Fatal(err)
	}
	mixedPath := filepath.Join(root, "provider-b", "revisions", aRevision.Revision+".json")
	if err := os.WriteFile(mixedPath, raw, 0o600); err != nil {
		t.Fatal(err)
	}

	if _, err := store.readRevisionLocked("provider-b", aRevision.Revision); err == nil {
		t.Fatal("mixed-provider revision was accepted")
	}
	// Also exercise it through a real restore call, not just the internal
	// reader, with a correct expected-revision so the only reason this can
	// fail is the mixed-provider revision itself.
	if _, err := store.Restore("provider-b", aRevision.Revision, bRevision.Revision); err == nil {
		t.Fatal("restore accepted a mixed-provider revision id")
	}
}

// TestSaveOverrideKeepsSparsePayloadFollowingBaselineUpdates pins the C5
// fix: disabling (or otherwise partially overriding) a provider must not
// freeze the entire resolved definition into the override. Before this fix,
// the disable handler saved the *whole* currently-effective definition with
// enabled flipped, so any field the user never touched (launch, models, ...)
// stopped tracking later embedded/distribution updates forever.
func TestSaveOverrideKeepsSparsePayloadFollowingBaselineUpdates(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	baseline := Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude-v1"}}
	disabled := false
	if _, err := store.SaveOverride("claude", Definition{ID: "claude", Enabled: &disabled}, baseline, "", "delete"); err != nil {
		t.Fatal(err)
	}
	definitions, diagnostics, err := store.LoadOverrides()
	if err != nil || len(diagnostics) != 0 || len(definitions) != 1 {
		t.Fatalf("LoadOverrides = %#v, %#v, %v", definitions, diagnostics, err)
	}
	override := definitions[0]
	if override.Launch != nil {
		t.Fatalf("sparse enabled-only override captured launch too: %#v", override.Launch)
	}
	if override.DisplayName != "" {
		t.Fatalf("sparse enabled-only override captured display_name too: %q", override.DisplayName)
	}
	if override.Enabled == nil || *override.Enabled {
		t.Fatal("override did not carry the enabled:false it was asked to set")
	}

	// Merging this sparse override against an UPDATED baseline (a later
	// embedded/distribution definition change) must pick up the update for
	// every field the override never touched, while the override's own
	// field (enabled) still applies.
	updatedBaseline := Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude-v2"}}
	registry, diags := Build(Layers{Embedded: []Definition{updatedBaseline}, Overrides: []Definition{override}}, DefaultAdapterCatalog())
	if len(diags) != 0 {
		t.Fatalf("Build diagnostics = %#v", diags)
	}
	effective, ok := registry.Lookup("claude")
	if !ok {
		t.Fatal("claude missing from registry")
	}
	if effective.Launch == nil || effective.Launch.Executable != "claude-v2" {
		t.Fatalf("effective launch = %#v, want claude-v2 (base update should have flowed through)", effective.Launch)
	}
	if effective.Enabled == nil || *effective.Enabled {
		t.Fatalf("effective enabled = %#v, want false (override should still apply)", effective.Enabled)
	}
}

// TestSaveOverrideAccumulatesAcrossMultiplePartialSaves pins that
// successive partial saves (e.g. an edit, then later a disable) merge onto
// each other rather than each wholly replacing the last — losing an
// earlier field customization when a later, unrelated field gets
// overridden would itself be a "did not track intent correctly" bug.
func TestSaveOverrideAccumulatesAcrossMultiplePartialSaves(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	baseline := Definition{SchemaVersion: 1, ID: "claude", DisplayName: "Claude", Launch: &LaunchDefinition{Executable: "claude"}}
	first, err := store.SaveOverride("claude", Definition{ID: "claude", DisplayName: "My Claude"}, baseline, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	disabled := false
	if _, err := store.SaveOverride("claude", Definition{ID: "claude", Enabled: &disabled}, baseline, first.Revision, "delete"); err != nil {
		t.Fatal(err)
	}
	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Payload.DisplayName != "My Claude" {
		t.Fatalf("disable lost the earlier display_name override: %#v", current.Payload)
	}
	if current.Payload.Enabled == nil || *current.Payload.Enabled {
		t.Fatalf("disable did not persist enabled:false: %#v", current.Payload)
	}
	if current.Payload.Launch != nil {
		t.Fatalf("neither save touched launch, but it ended up set: %#v", current.Payload.Launch)
	}
}

func TestHistoryStoreSymlinkProviderDirDoesNotEscapeRoot(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	if err := os.MkdirAll(root, 0o700); err != nil {
		t.Fatal(err)
	}
	outside := t.TempDir()
	link := filepath.Join(root, "example")
	if err := os.Symlink(outside, link); err != nil {
		t.Skipf("symlink not available: %v", err)
	}
	store, err := NewHistoryStore(root, filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("example", testOverride("example", "Example"), Definition{}, "", "edit"); err == nil {
		t.Fatal("symlink provider dir was accepted")
	}
	entries, err := os.ReadDir(outside)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Fatalf("override escaped store root into %#v", entries)
	}
}

func TestHistoryStoreLastVerifiedRevision(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}

	if _, found, err := store.LastVerifiedRevision("claude"); err != nil || found {
		t.Fatalf("LastVerifiedRevision with no revisions = found %v, err %v", found, err)
	}

	first, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(time.Millisecond)
	second, err := store.SaveOverride("claude", testOverride("claude", "Changed"), Definition{}, first.Revision, "edit")
	if err != nil {
		t.Fatal(err)
	}
	if first.Revision == second.Revision {
		t.Fatal("successive revisions reused the same id")
	}

	latest, found, err := store.LastVerifiedRevision("claude")
	if err != nil || !found {
		t.Fatalf("LastVerifiedRevision after two saves = found %v, err %v", found, err)
	}
	if latest.Revision != second.Revision {
		t.Fatalf("LastVerifiedRevision = %q, want newest revision %q", latest.Revision, second.Revision)
	}

	secondPath := filepath.Join(root, "claude", "revisions", second.Revision+".json")
	if err := os.WriteFile(secondPath, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	fallback, found, err := store.LastVerifiedRevision("claude")
	if err != nil || !found {
		t.Fatalf("LastVerifiedRevision with newest corrupt = found %v, err %v", found, err)
	}
	if fallback.Revision != first.Revision {
		t.Fatalf("LastVerifiedRevision fell back to %q, want older revision %q", fallback.Revision, first.Revision)
	}
}

// corruptCurrentRevisionFile overwrites the on-disk revision file that HEAD
// currently points to with content that fails readRevision's schema check
// (not a missing-file error), so callers see a genuine "HEAD is broken" case
// rather than the "no override exists yet" case os.IsNotExist reports.
func corruptCurrentRevisionFile(t *testing.T, root, providerID, revision string) {
	t.Helper()
	path := filepath.Join(root, providerID, "revisions", revision+".json")
	if err := os.WriteFile(path, []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestHistoryStoreRecoverHeadToChosenRevision(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	first, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(time.Millisecond)
	second, err := store.SaveOverride("claude", testOverride("claude", "Changed"), Definition{}, first.Revision, "edit")
	if err != nil {
		t.Fatal(err)
	}
	corruptCurrentRevisionFile(t, root, "claude", second.Revision)

	recovered, err := store.RecoverHead("claude", first.Revision)
	if err != nil {
		t.Fatal(err)
	}
	if recovered.Payload.DisplayName != "Claude" {
		t.Fatalf("recovered payload = %#v", recovered.Payload)
	}
	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != recovered.Revision {
		t.Fatalf("Current after recovery = %q, want %q", current.Revision, recovered.Revision)
	}
}

func TestHistoryStoreRecoverHeadToBaseWithEmptyRevision(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	saved, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	corruptCurrentRevisionFile(t, root, "claude", saved.Revision)

	recovered, err := store.RecoverHead("claude", "")
	if err != nil {
		t.Fatal(err)
	}
	if recovered.Payload.DisplayName != "" {
		t.Fatalf("recovered payload should be override-free, got %#v", recovered.Payload)
	}
	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != recovered.Revision {
		t.Fatalf("Current after recovery = %q, want %q", current.Revision, recovered.Revision)
	}
}

func TestHistoryStoreRecoverHeadRejectsHealthyHead(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	saved, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.RecoverHead("claude", ""); err == nil {
		t.Fatal("RecoverHead accepted a healthy HEAD")
	}
	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != saved.Revision {
		t.Fatalf("Current changed after rejected RecoverHead: %q, want %q", current.Revision, saved.Revision)
	}
}

func TestHistoryStoreRecoverHeadQuarantinesCorruptHead(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	saved, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	corruptCurrentRevisionFile(t, root, "claude", saved.Revision)

	if _, err := store.RecoverHead("claude", ""); err != nil {
		t.Fatal(err)
	}
	entries, err := os.ReadDir(filepath.Join(backups, "quarantine"))
	if err != nil || len(entries) == 0 {
		t.Fatalf("quarantine entries = %#v, %v", entries, err)
	}
}

func TestHistoryStoreNeedsRecovery(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}

	if needs, err := store.NeedsRecovery("claude"); err != nil || needs {
		t.Fatalf("NeedsRecovery with no HEAD = %v, %v, want false, nil", needs, err)
	}

	saved, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if needs, err := store.NeedsRecovery("claude"); err != nil || needs {
		t.Fatalf("NeedsRecovery with healthy HEAD = %v, %v, want false, nil", needs, err)
	}

	if err := os.WriteFile(filepath.Join(root, "claude", "HEAD"), []byte("missing-revision\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if needs, err := store.NeedsRecovery("claude"); err != nil || !needs {
		t.Fatalf("NeedsRecovery with HEAD pointing to missing revision = %v, %v, want true, nil", needs, err)
	}

	if err := os.WriteFile(filepath.Join(root, "claude", "HEAD"), []byte(saved.Revision), 0o600); err != nil {
		t.Fatal(err)
	}
	corruptCurrentRevisionFile(t, root, "claude", saved.Revision)
	if needs, err := store.NeedsRecovery("claude"); err != nil || !needs {
		t.Fatalf("NeedsRecovery with corrupt revision = %v, %v, want true, nil", needs, err)
	}
}

func TestHistoryStoreRecoverHeadFromMissingRevision(t *testing.T) {
	root := filepath.Join(t.TempDir(), "overrides")
	backups := filepath.Join(t.TempDir(), "backups")
	store, err := NewHistoryStore(root, backups)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.SaveOverride("claude", testOverride("claude", "Claude"), Definition{}, "", "edit"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "claude", "HEAD"), []byte("missing-revision\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	recovered, err := store.RecoverHead("claude", "")
	if err != nil {
		t.Fatal(err)
	}
	current, err := store.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	if current.Revision != recovered.Revision {
		t.Fatalf("Current after recovery = %q, want %q", current.Revision, recovered.Revision)
	}
}

func TestHistoryStoreSaveEffectiveOverrideKeepsOnlyChangedFields(t *testing.T) {
	store, err := NewHistoryStore(filepath.Join(t.TempDir(), "overrides"), filepath.Join(t.TempDir(), "backups"))
	if err != nil {
		t.Fatal(err)
	}
	base := Definition{
		SchemaVersion: CurrentSchemaVersion,
		ID:            "codex", DisplayName: "Codex",
		Launch: &LaunchDefinition{Executable: "codex-v1", Args: []string{"run"}},
	}
	desired := base
	desired.DisplayName = "My Codex"
	record, err := store.SaveEffectiveOverride("codex", desired, base, "", "edit")
	if err != nil {
		t.Fatal(err)
	}
	if record.Payload.DisplayName != "My Codex" {
		t.Fatalf("display name override = %q", record.Payload.DisplayName)
	}
	if record.Payload.Launch != nil {
		t.Fatalf("unchanged launch was frozen into override: %#v", record.Payload.Launch)
	}
	updatedBase := base
	updatedBase.Launch = &LaunchDefinition{Executable: "codex-v2", Args: []string{"new-run"}}
	merged, err := mergeDefinitionValues(updatedBase, record.Payload)
	if err != nil {
		t.Fatal(err)
	}
	if merged.Launch == nil || merged.Launch.Executable != "codex-v2" {
		t.Fatalf("unmodified launch did not follow updated base: %#v", merged.Launch)
	}
	if merged.DisplayName != "My Codex" {
		t.Fatalf("modified display name was lost: %q", merged.DisplayName)
	}
}

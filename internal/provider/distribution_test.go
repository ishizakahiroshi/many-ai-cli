package provider

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestVerifyDistributionBundleSeparatesVerificationFromAcceptance(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	definition := Definition{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}
	payload := BuildDistributionPayload([]Definition{definition}, "2026.09.14", "2026-09-14T00:00:00Z", "0.8.0")
	bundle, err := SignDistributionPayload(payload, "test-key", privateKey)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(bundle)
	verified, digest, err := VerifyDistributionBundle(raw, map[string]ed25519.PublicKey{"test-key": publicKey}, "0.8.1")
	if err != nil || verified.Payload.CatalogVersion != payload.CatalogVersion || digest == "" {
		t.Fatalf("VerifyDistributionBundle = %#v, %q, %v", verified, digest, err)
	}
	store, err := NewDistributionStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	// SaveDownloaded takes the exact bytes VerifyDistributionBundle checked,
	// never a re-marshal of the decoded struct: Accept later re-hashes these
	// bytes and must get the same digest back.
	if err := store.SaveDownloaded(raw, digest); err != nil {
		t.Fatal(err)
	}
}

func TestFetchDistributionRejectsNonOfficialEndpointsBeforeNetwork(t *testing.T) {
	for _, rawURL := range []string{"http://raw.githubusercontent.com/example.json", "https://example.com/provider.json"} {
		if _, err := FetchDistribution(context.Background(), rawURL, nil); err == nil {
			t.Fatalf("FetchDistribution(%q) accepted an untrusted endpoint", rawURL)
		}
	}
}

func TestVerifyDistributionBundleRejectsTamperingAndUnknownKeys(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	payload := BuildDistributionPayload([]Definition{{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}}, "v1", "now", "")
	bundle, err := SignDistributionPayload(payload, "test-key", privateKey)
	if err != nil {
		t.Fatal(err)
	}
	bundle.Payload.Definitions[0].DisplayName = "Tampered"
	raw, _ := json.Marshal(bundle)
	if _, _, err := VerifyDistributionBundle(raw, map[string]ed25519.PublicKey{"test-key": publicKey}, ""); err == nil {
		t.Fatal("tampered distribution was accepted")
	}
	bundle.Payload.Definitions[0].DisplayName = "Example"
	bundle.KeyID = "unknown"
	raw, _ = json.Marshal(bundle)
	if _, _, err := VerifyDistributionBundle(raw, map[string]ed25519.PublicKey{"test-key": publicKey}, ""); err == nil {
		t.Fatal("unknown distribution key was accepted")
	}
}

func TestDiffDistributionMarksConflictsWithoutResolvingThem(t *testing.T) {
	current := []Definition{{SchemaVersion: 1, ID: "example", DisplayName: "Old", Launch: &LaunchDefinition{Executable: "old"}}}
	candidate := []Definition{{SchemaVersion: 1, ID: "example", DisplayName: "New", Launch: &LaunchDefinition{Executable: "new"}}}
	override := []Definition{{SchemaVersion: 1, ID: "example", DisplayName: "Mine", Launch: &LaunchDefinition{Executable: "mine"}}}
	diff := DiffDistribution(current, candidate, override)
	if len(diff) != 1 || diff[0].Status != "changed" {
		t.Fatalf("diff = %#v", diff)
	}
	var display DistributionFieldDiff
	for _, change := range diff[0].Changes {
		if change.Field == "display_name" {
			display = change
		}
	}
	if !display.Conflict || display.Override != "Mine" || display.Candidate != "New" {
		t.Fatalf("display_name change = %#v, want unresolved conflict", display)
	}
}

// signTestBundle builds+signs a single-provider distribution payload and
// returns its raw wire bytes and content digest, ready for SaveDownloaded.
func signTestBundle(t *testing.T, catalogVersion, displayName string, privateKey ed25519.PrivateKey, keyID string, publicKey ed25519.PublicKey) ([]byte, string) {
	t.Helper()
	payload := BuildDistributionPayload([]Definition{{SchemaVersion: 1, ID: "example", DisplayName: displayName, Launch: &LaunchDefinition{Executable: "example"}}}, catalogVersion, "now", "")
	bundle, err := SignDistributionPayload(payload, keyID, privateKey)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(bundle)
	if err != nil {
		t.Fatal(err)
	}
	_, digest, err := VerifyDistributionBundle(raw, map[string]ed25519.PublicKey{keyID: publicKey}, "")
	if err != nil {
		t.Fatal(err)
	}
	return raw, digest
}

func TestDistributionStoreAcceptRollbackKeepsPreviousPointer(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}
	store, err := NewDistributionStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	firstRaw, firstDigest := signTestBundle(t, "v1", "One", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(firstRaw, firstDigest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(firstDigest, trusted, ""); err != nil {
		t.Fatal(err)
	}
	secondRaw, secondDigest := signTestBundle(t, "v2", "Two", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(secondRaw, secondDigest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(secondDigest, trusted, ""); err != nil {
		t.Fatal(err)
	}
	rolled, err := store.Rollback()
	if err != nil {
		t.Fatal(err)
	}
	if rolled.Digest != firstDigest {
		t.Fatalf("rollback digest = %s, want %s", rolled.Digest, firstDigest)
	}
	loaded, err := store.LoadAcceptedBundle()
	if err != nil {
		t.Fatal(err)
	}
	if loaded.Payload.CatalogVersion != "v1" {
		t.Fatalf("accepted after rollback = %#v", loaded.Payload)
	}
}

func TestDistributionStoreAcceptRejectsPathTraversalAndReservedNames(t *testing.T) {
	store, err := NewDistributionStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	badDigests := []string{
		"../escape",
		"../../etc/passwd",
		`..\escape`,
		"/etc/passwd",
		`C:\Windows\System32\config`,
		"con",
		"CON.json",
		"nul",
		"lpt1",
		"",
		".",
		"..",
	}
	for _, digest := range badDigests {
		if _, err := store.Accept(digest, map[string]ed25519.PublicKey{}, ""); err == nil {
			t.Fatalf("Accept(%q) was accepted", digest)
		}
	}
	status, err := store.Status()
	if err != nil {
		t.Fatal(err)
	}
	if status.State != "none" {
		t.Fatalf("rejected accept mutated status = %#v", status)
	}
}

func TestDistributionStoreAcceptRejectsUnknownKeyAndMissingDownload(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	store, err := NewDistributionStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	raw, digest := signTestBundle(t, "v1", "One", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(raw, digest); err != nil {
		t.Fatal(err)
	}
	// Never downloaded: Accept must not synthesize a bundle from nothing.
	otherPublicKey, otherPrivateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	_, neverDownloadedDigest := signTestBundle(t, "v9", "Nope", otherPrivateKey, "test-key", otherPublicKey)
	if _, err := store.Accept(neverDownloadedDigest, map[string]ed25519.PublicKey{"test-key": publicKey}, ""); err == nil {
		t.Fatal("accept succeeded for a digest that was never downloaded")
	}
	// Downloaded, but the key isn't trusted at accept time (e.g. retired
	// between download and accept).
	if _, err := store.Accept(digest, map[string]ed25519.PublicKey{}, ""); err == nil {
		t.Fatal("accept succeeded against an empty trusted-key set")
	}
	if status, statusErr := store.Status(); statusErr != nil || status.State != "none" {
		t.Fatalf("rejected accepts mutated status = %#v, %v", status, statusErr)
	}
}

// TestDistributionStoreAcceptRejectsTamperedDownload pins the "Accept itself
// re-verifies" requirement: editing the cached downloaded/<digest>.json file
// after SaveDownloaded wrote it (so its bytes no longer match the digest, or
// no longer satisfy the signature) must make Accept fail, not silently
// promote whatever is now on disk under that name.
func TestDistributionStoreAcceptRejectsTamperedDownload(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}
	root := t.TempDir()
	store, err := NewDistributionStore(root)
	if err != nil {
		t.Fatal(err)
	}
	raw, digest := signTestBundle(t, "v1", "One", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(raw, digest); err != nil {
		t.Fatal(err)
	}
	tampered := append(append([]byte(nil), raw[:len(raw)-1]...), byte('0'))
	if err := os.WriteFile(filepath.Join(root, "downloaded", digest+".json"), tampered, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(digest, trusted, ""); err == nil {
		t.Fatal("accept succeeded against a tampered downloaded file")
	}
	if status, statusErr := store.Status(); statusErr != nil || status.State != "none" {
		t.Fatalf("rejected accept mutated status = %#v, %v", status, statusErr)
	}
}

// TestDistributionStoreLoadAcceptedBundleRejectsCorruptedContent pins
// "LoadAcceptedBundle re-verifies pointer vs. content": corrupting the
// accepted bundle file after a successful Accept must be caught on load,
// not returned as if it were still the accepted definition set.
func TestDistributionStoreLoadAcceptedBundleRejectsCorruptedContent(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}
	root := t.TempDir()
	store, err := NewDistributionStore(root)
	if err != nil {
		t.Fatal(err)
	}
	raw, digest := signTestBundle(t, "v1", "One", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(raw, digest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(digest, trusted, ""); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "accepted", digest+".json"), []byte("{not the same bytes}"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := store.LoadAcceptedBundle(); err == nil {
		t.Fatal("LoadAcceptedBundle accepted content that no longer matches its digest")
	}
}

// TestDistributionStoreRollbackRejectsBrokenPreviousBundle pins "Rollback
// verifies the previous target before swapping the pointer": if the bundle
// file previous.json points at is missing or corrupted, Rollback must fail
// before writing accepted.json, leaving the current accepted pointer intact.
func TestDistributionStoreRollbackRejectsBrokenPreviousBundle(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}
	root := t.TempDir()
	store, err := NewDistributionStore(root)
	if err != nil {
		t.Fatal(err)
	}
	firstRaw, firstDigest := signTestBundle(t, "v1", "One", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(firstRaw, firstDigest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(firstDigest, trusted, ""); err != nil {
		t.Fatal(err)
	}
	secondRaw, secondDigest := signTestBundle(t, "v2", "Two", privateKey, "test-key", publicKey)
	if err := store.SaveDownloaded(secondRaw, secondDigest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(secondDigest, trusted, ""); err != nil {
		t.Fatal(err)
	}
	// The rollback target (v1's bundle file) is gone by the time Rollback runs.
	if err := os.Remove(filepath.Join(root, "accepted", firstDigest+".json")); err != nil {
		t.Fatal(err)
	}
	beforeRollback, err := store.Status()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.Rollback(); err == nil {
		t.Fatal("rollback succeeded despite a missing previous bundle file")
	}
	afterRollback, err := store.Status()
	if err != nil {
		t.Fatal(err)
	}
	if afterRollback != beforeRollback {
		t.Fatalf("failed rollback changed accepted status: before=%#v after=%#v", beforeRollback, afterRollback)
	}
	loaded, err := store.LoadAcceptedBundle()
	if err != nil || loaded.Payload.CatalogVersion != "v2" {
		t.Fatalf("failed rollback left an unreadable/wrong accepted bundle: %#v, %v", loaded.Payload, err)
	}
}

func TestVerifyDistributionBundleRejectsRetiredKeys(t *testing.T) {
	oldPub, oldPriv, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	newPub, newPriv, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	payload := BuildDistributionPayload([]Definition{{SchemaVersion: 1, ID: "example", DisplayName: "Example", Launch: &LaunchDefinition{Executable: "example"}}}, "v1", "now", "")
	oldBundle, err := SignDistributionPayload(payload, "old-key", oldPriv)
	if err != nil {
		t.Fatal(err)
	}
	newBundle, err := SignDistributionPayload(payload, "new-key", newPriv)
	if err != nil {
		t.Fatal(err)
	}
	oldRaw, _ := json.Marshal(oldBundle)
	newRaw, _ := json.Marshal(newBundle)
	trusted := map[string]ed25519.PublicKey{"old-key": oldPub, "new-key": newPub}
	if _, _, err := VerifyDistributionBundle(oldRaw, trusted, ""); err != nil {
		t.Fatal(err)
	}
	retired := map[string]ed25519.PublicKey{"new-key": newPub}
	if _, _, err := VerifyDistributionBundle(oldRaw, retired, ""); err == nil {
		t.Fatal("retired key was still trusted")
	}
	if _, _, err := VerifyDistributionBundle(newRaw, retired, ""); err != nil {
		t.Fatal(err)
	}
}

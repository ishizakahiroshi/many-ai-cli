package provider

import (
	"context"
	"crypto/ed25519"
	"encoding/json"
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
	if err := store.SaveDownloaded(verified, digest); err != nil {
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

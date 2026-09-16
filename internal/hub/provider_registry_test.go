package hub

import (
	"crypto/ed25519"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func TestBuildProviderRegistryIncludesEmbeddedAndLegacyProviders(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent"}}}
	registry, diagnostics, err := buildProviderRegistry(cfg)
	if err != nil {
		t.Fatalf("buildProviderRegistry: %v", err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("buildProviderRegistry diagnostics = %#v", diagnostics)
	}
	if _, ok := registry.Lookup("claude"); !ok {
		t.Fatal("embedded claude is not in the registry")
	}
	custom, ok := registry.Lookup("my-cli")
	if !ok || custom.Launch == nil || custom.Launch.Executable != "my-cli" {
		t.Fatalf("legacy custom definition = %#v, %v", custom, ok)
	}
	if custom.EffectiveSource.Origin != "legacy" {
		t.Fatalf("custom origin = %q, want legacy", custom.EffectiveSource.Origin)
	}
}

func TestValidSpawnProviderUsesRegistrySnapshot(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{{ID: "my-cli", Command: "my-cli"}}}
	s := &Server{cfg: cfg}
	if !s.validSpawnProvider("claude") || !s.validSpawnProvider("my-cli") || s.validSpawnProvider("not-registered") {
		t.Fatal("validSpawnProvider did not use the registry's effective entries")
	}
}

func signAndAcceptDistribution(t *testing.T, store *provider.DistributionStore, catalogVersion, definitionID string, privateKey ed25519.PrivateKey, publicKey ed25519.PublicKey) provider.DistributionStatus {
	t.Helper()
	payload := provider.BuildDistributionPayload([]provider.Definition{
		{SchemaVersion: 1, ID: definitionID, DisplayName: definitionID, Launch: &provider.LaunchDefinition{Executable: definitionID}},
	}, catalogVersion, "now", "")
	bundle, err := provider.SignDistributionPayload(payload, "test-key", privateKey)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(bundle)
	if err != nil {
		t.Fatal(err)
	}
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}
	_, digest, err := provider.VerifyDistributionBundle(raw, trusted, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := store.SaveDownloaded(raw, digest); err != nil {
		t.Fatal(err)
	}
	status, err := store.Accept(digest, trusted, "")
	if err != nil {
		t.Fatal(err)
	}
	return status
}

// TestReloadProviderRegistryAppliesAcceptedDistribution pins the C4 fix:
// buildProviderRegistryLayers previously never received the accepted
// distribution's definitions at all (Layers.AcceptedDistribution stayed
// nil forever), so accepting a catalog had no effect on what a session
// spawn or the provider list actually resolved.
func TestReloadProviderRegistryAppliesAcceptedDistribution(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	store, err := provider.NewDistributionStore(filepath.Join(t.TempDir(), "provider-distributions"))
	if err != nil {
		t.Fatal(err)
	}
	signAndAcceptDistribution(t, store, "v1", "dist-cli", privateKey, publicKey)

	s := newTestServer()
	s.distributionStore = store
	diagnostics, err := s.reloadProviderRegistry()
	if err != nil {
		t.Fatalf("reloadProviderRegistry: %v, diagnostics=%#v", err, diagnostics)
	}
	definition, ok := s.providerRegistrySnapshot().Lookup("dist-cli")
	if !ok {
		t.Fatal("accepted distribution definition was not applied to the registry")
	}
	if definition.EffectiveSource.Origin != provider.OriginDistribution {
		t.Fatalf("dist-cli origin = %q, want distribution", definition.EffectiveSource.Origin)
	}
}

// TestAcceptedDistributionSurvivesSimulatedRestart exercises the same
// startup path NewServer uses (load accepted distribution, then build
// registry layers with it) without actually restarting a process, pinning
// "Hub 再起動後も accepted distribution が再適用される".
func TestAcceptedDistributionSurvivesSimulatedRestart(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(t.TempDir(), "provider-distributions")
	store, err := provider.NewDistributionStore(root)
	if err != nil {
		t.Fatal(err)
	}
	signAndAcceptDistribution(t, store, "v1", "dist-cli", privateKey, publicKey)

	// A fresh DistributionStore handle over the same root simulates a
	// process restart reading the same on-disk state, the same way
	// NewServer's startup branch calls loadAcceptedDistributionDefinitions.
	restarted, err := provider.NewDistributionStore(root)
	if err != nil {
		t.Fatal(err)
	}
	accepted, diagnostics := loadAcceptedDistributionDefinitions(restarted)
	if len(diagnostics) != 0 {
		t.Fatalf("unexpected diagnostics after restart: %#v", diagnostics)
	}
	registry, buildDiagnostics, err := buildProviderRegistryLayers(&config.Config{}, nil, nil, accepted)
	if err != nil {
		t.Fatalf("buildProviderRegistryLayers: %v, diagnostics=%#v", err, buildDiagnostics)
	}
	if _, ok := registry.Lookup("dist-cli"); !ok {
		t.Fatal("accepted distribution did not survive a simulated restart")
	}
}

// TestProviderDistributionRollbackReloadsRegistry pins "rollback 後に
// Registry を reload する": the previous implementation swapped the on-disk
// accepted pointer back but never told the cached Registry, so a rolled
// back distribution stayed resolvable until an unrelated reload happened.
func TestProviderDistributionRollbackReloadsRegistry(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	store, err := provider.NewDistributionStore(filepath.Join(t.TempDir(), "provider-distributions"))
	if err != nil {
		t.Fatal(err)
	}
	signAndAcceptDistribution(t, store, "v1", "dist-cli-one", privateKey, publicKey)
	signAndAcceptDistribution(t, store, "v2", "dist-cli-two", privateKey, publicKey)

	s := newTestServer()
	s.cfg.Token = "test-token"
	s.distributionStore = store
	if _, err := s.reloadProviderRegistry(); err != nil {
		t.Fatal(err)
	}
	if _, ok := s.providerRegistrySnapshot().Lookup("dist-cli-two"); !ok {
		t.Fatal("v2 was not applied before rollback")
	}

	req := httptest.NewRequest(http.MethodPost, "/api/provider-distributions/rollback?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributionRollback(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("rollback status = %d, body=%s", resp.Code, resp.Body.String())
	}

	if _, ok := s.providerRegistrySnapshot().Lookup("dist-cli-two"); ok {
		t.Fatal("rollback did not reload the registry: v2 definition is still resolvable")
	}
	if _, ok := s.providerRegistrySnapshot().Lookup("dist-cli-one"); !ok {
		t.Fatal("rollback did not restore v1's definition in the reloaded registry")
	}
}

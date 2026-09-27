package hub

import (
	"crypto/ed25519"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/provider"
)

func TestProviderDistributionCheckIsFailClosedWithoutKeys(t *testing.T) {
	s := newProviderAPITestServer(t)
	req := httptest.NewRequest(http.MethodPost, "/api/provider-distributions/check?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusServiceUnavailable {
		t.Fatalf("POST check status = %d, want 503", resp.Code)
	}
}

func TestProviderDistributionAcceptIsFailClosedWithoutKeys(t *testing.T) {
	s := newProviderAPITestServer(t)
	req := httptest.NewRequest(http.MethodPost, "/api/provider-distributions/accept?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusServiceUnavailable {
		t.Fatalf("POST accept status = %d, want 503", resp.Code)
	}
}

func TestProviderDistributionStatusReportsDisabled(t *testing.T) {
	s := newProviderAPITestServer(t)
	req := httptest.NewRequest(http.MethodGet, "/api/provider-distributions/status?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("GET status = %d, want 200", resp.Code)
	}
}

func TestProviderDistributionDiffRequiresDigest(t *testing.T) {
	s := newProviderAPITestServer(t)
	store, err := provider.NewDistributionStore(filepath.Join(t.TempDir(), "provider-distributions"))
	if err != nil {
		t.Fatal(err)
	}
	s.distributionStore = store
	req := httptest.NewRequest(http.MethodGet, "/api/provider-distributions/diff?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusBadRequest {
		t.Fatalf("diff without digest status = %d, want 400, body=%s", resp.Code, resp.Body.String())
	}
}

func TestProviderDistributionDiffUnknownCandidateIs404(t *testing.T) {
	s := newProviderAPITestServer(t)
	store, err := provider.NewDistributionStore(filepath.Join(t.TempDir(), "provider-distributions"))
	if err != nil {
		t.Fatal(err)
	}
	s.distributionStore = store
	req := httptest.NewRequest(http.MethodGet, "/api/provider-distributions/diff?token=test-token&digest=neverdownloaded", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusNotFound {
		t.Fatalf("diff for unknown digest status = %d, want 404, body=%s", resp.Code, resp.Body.String())
	}
}

// TestProviderDistributionDiffComparesCurrentAgainstCandidate pins the C4
// fix: the previous implementation passed the accepted bundle as both
// "current" and "candidate" to DiffDistribution, so the diff was always
// empty. A real candidate (downloaded but not accepted) must show up as
// "added" against an accepted bundle that doesn't have it yet.
func TestProviderDistributionDiffComparesCurrentAgainstCandidate(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	s := newProviderAPITestServer(t)
	store, err := provider.NewDistributionStore(filepath.Join(t.TempDir(), "provider-distributions"))
	if err != nil {
		t.Fatal(err)
	}
	s.distributionStore = store
	trusted := map[string]ed25519.PublicKey{"test-key": publicKey}

	currentPayload := provider.BuildDistributionPayload([]provider.Definition{
		{SchemaVersion: 1, ID: "dist-current", DisplayName: "Current", Launch: &provider.LaunchDefinition{Executable: "dist-current"}},
	}, "v1", "now", "")
	currentBundle, err := provider.SignDistributionPayload(currentPayload, "test-key", privateKey)
	if err != nil {
		t.Fatal(err)
	}
	currentRaw, err := json.Marshal(currentBundle)
	if err != nil {
		t.Fatal(err)
	}
	_, currentDigest, err := provider.VerifyDistributionBundle(currentRaw, trusted, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := store.SaveDownloaded(currentRaw, currentDigest); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Accept(currentDigest, trusted, ""); err != nil {
		t.Fatal(err)
	}

	candidatePayload := provider.BuildDistributionPayload([]provider.Definition{
		{SchemaVersion: 1, ID: "dist-current", DisplayName: "Current", Launch: &provider.LaunchDefinition{Executable: "dist-current"}},
		{SchemaVersion: 1, ID: "dist-new", DisplayName: "New", Launch: &provider.LaunchDefinition{Executable: "dist-new"}},
	}, "v2", "now", "")
	candidateBundle, err := provider.SignDistributionPayload(candidatePayload, "test-key", privateKey)
	if err != nil {
		t.Fatal(err)
	}
	candidateRaw, err := json.Marshal(candidateBundle)
	if err != nil {
		t.Fatal(err)
	}
	_, candidateDigest, err := provider.VerifyDistributionBundle(candidateRaw, trusted, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := store.SaveDownloaded(candidateRaw, candidateDigest); err != nil {
		t.Fatal(err)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/provider-distributions/diff?token=test-token&digest="+candidateDigest, nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderDistributions(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("diff status = %d, body=%s", resp.Code, resp.Body.String())
	}
	var body struct {
		Diff []provider.DistributionProviderDiff `json:"diff"`
	}
	if err := json.Unmarshal(resp.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	statuses := map[string]string{}
	for _, item := range body.Diff {
		statuses[item.ID] = item.Status
	}
	if statuses["dist-current"] != "unchanged" {
		t.Fatalf("dist-current status = %q, want unchanged", statuses["dist-current"])
	}
	if statuses["dist-new"] != "added" {
		t.Fatalf("dist-new status = %q, want added (diff = %#v)", statuses["dist-new"], body.Diff)
	}
}

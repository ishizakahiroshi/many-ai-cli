package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/provider"
)

func newProviderAPITestServer(t *testing.T) *Server {
	t.Helper()
	s := newTestServer()
	s.cfg.Token = "test-token"
	store, err := provider.NewFileStore(filepath.Join(t.TempDir(), "providers.d"))
	if err != nil {
		t.Fatal(err)
	}
	s.providerStore = store
	registry, diagnostics, err := buildProviderRegistry(s.cfg)
	if err != nil || len(diagnostics) != 0 {
		t.Fatalf("build provider registry: %v, %#v", err, diagnostics)
	}
	s.providers = registry
	return s
}

func TestProviderListAPIUsesRegistryAndProtectsSecrets(t *testing.T) {
	s := newProviderAPITestServer(t)
	req := httptest.NewRequest(http.MethodGet, "/api/providers?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviders(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("GET /api/providers status = %d, want 200", resp.Code)
	}
	var body struct {
		Providers []provider.Summary `json:"providers"`
	}
	if err := json.Unmarshal(resp.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if len(body.Providers) != 7 || body.Providers[0].ID != "claude" {
		t.Fatalf("providers = %#v, want embedded registry order", body.Providers)
	}
	if bytes.Contains(resp.Body.Bytes(), []byte("test-token")) {
		t.Fatal("provider list leaked the Hub token")
	}
}

func TestProviderCreatePatchDeleteAPIUsesRevision(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition := provider.Definition{
		SchemaVersion: 1,
		ID:            "example-cli",
		DisplayName:   "Example CLI",
		Launch:        &provider.LaunchDefinition{Executable: "example-cli"},
	}
	createReq := httptest.NewRequest(http.MethodPost, "/api/providers?token=test-token", bytes.NewReader(mustJSON(t, definition)))
	createReq.Header.Set("Origin", "http://127.0.0.1:47777")
	createReq.Host = "127.0.0.1:47777"
	createResp := httptest.NewRecorder()
	s.handleProviders(createResp, createReq)
	if createResp.Code != http.StatusCreated {
		t.Fatalf("POST /api/providers status = %d, body=%s", createResp.Code, createResp.Body.String())
	}
	registry := s.providerRegistrySnapshot()
	if _, ok := registry.Lookup("example-cli"); !ok {
		t.Fatal("created provider was not reloaded")
	}
	expected := registry.Revision()
	definition.DisplayName = "Updated CLI"
	patchBody := map[string]any{"expected_revision": expected, "definition": definition}
	patchReq := httptest.NewRequest(http.MethodPatch, "/api/providers/example-cli?token=test-token", bytes.NewReader(mustJSON(t, patchBody)))
	patchReq.Header.Set("Origin", "http://127.0.0.1:47777")
	patchReq.Host = "127.0.0.1:47777"
	patchResp := httptest.NewRecorder()
	s.handleProviderRoute(patchResp, patchReq)
	if patchResp.Code != http.StatusOK {
		t.Fatalf("PATCH provider status = %d, body=%s", patchResp.Code, patchResp.Body.String())
	}
	staleReq := httptest.NewRequest(http.MethodPatch, "/api/providers/example-cli?token=test-token", bytes.NewReader(mustJSON(t, patchBody)))
	staleReq.Header.Set("Origin", "http://127.0.0.1:47777")
	staleReq.Host = "127.0.0.1:47777"
	staleResp := httptest.NewRecorder()
	s.handleProviderRoute(staleResp, staleReq)
	if staleResp.Code != http.StatusConflict {
		t.Fatalf("stale PATCH status = %d, want 409", staleResp.Code)
	}
	deleteReq := httptest.NewRequest(http.MethodDelete, "/api/providers/example-cli?token=test-token&expected_revision="+s.providerRegistrySnapshot().Revision(), nil)
	deleteReq.Header.Set("Origin", "http://127.0.0.1:47777")
	deleteReq.Host = "127.0.0.1:47777"
	deleteResp := httptest.NewRecorder()
	s.handleProviderRoute(deleteResp, deleteReq)
	if deleteResp.Code != http.StatusOK {
		t.Fatalf("DELETE provider status = %d, body=%s", deleteResp.Code, deleteResp.Body.String())
	}
}

func mustJSON(t *testing.T, value any) []byte {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

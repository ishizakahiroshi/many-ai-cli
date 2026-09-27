package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/provider"
)

// newProviderRollbackDrillTestServer is like newProviderRecoveryTestServer,
// but also returns the HistoryStore's backup root so a test can inspect
// ~/.many-ai-cli/backups/providers/quarantine (the drill's own tree, under
// t.TempDir()) after a corrupt HEAD is discovered.
func newProviderRollbackDrillTestServer(t *testing.T) (*Server, string, string) {
	t.Helper()
	s := newTestServer()
	s.cfg.Token = "test-token"
	store, err := provider.NewFileStore(filepath.Join(t.TempDir(), "providers.d"))
	if err != nil {
		t.Fatal(err)
	}
	s.providerStore = store
	historyRoot := filepath.Join(t.TempDir(), "history.d")
	backupRoot := filepath.Join(t.TempDir(), "backups.d")
	history, err := provider.NewHistoryStore(historyRoot, backupRoot)
	if err != nil {
		t.Fatal(err)
	}
	s.historyStore = history
	registry, diagnostics, err := buildProviderRegistry(s.cfg)
	if err != nil || len(diagnostics) != 0 {
		t.Fatalf("build provider registry: %v, %#v", err, diagnostics)
	}
	s.providers = registry
	return s, historyRoot, backupRoot
}

// TestProviderRollbackDrillCorruptHead is C12 C7's scenario 1: it drives a
// built-in provider's HEAD file into a state HistoryStore cannot resolve
// (not a broken revision HEAD merely points at, but the HEAD pointer file
// itself) purely through the Hub's HTTP surface, then checks that recovery
// is actually possible end to end — GET .../recovery reports the break,
// POST .../recovery repairs it to a chosen prior revision, a follow-up GET
// on the provider confirms the repaired value took effect, and the broken
// HEAD landed in the quarantine tree instead of being silently discarded.
func TestProviderRollbackDrillCorruptHead(t *testing.T) {
	s, historyRoot, backupRoot := newProviderRollbackDrillTestServer(t)

	definition, ok := s.providerRegistrySnapshot().Lookup("codex")
	if !ok {
		t.Fatal("codex is not registered")
	}
	definition.Definition.DisplayName = "Codex first edit"
	if resp := patchProviderRequest(t, s, "codex", "", definition.Definition); resp.Code != http.StatusOK {
		t.Fatalf("first edit status = %d, body=%s", resp.Code, resp.Body.String())
	}
	first, err := s.historyStore.Current("codex")
	if err != nil {
		t.Fatal(err)
	}

	definition.Definition.DisplayName = "Codex second edit"
	if resp := patchProviderRequest(t, s, "codex", first.Revision, definition.Definition); resp.Code != http.StatusOK {
		t.Fatalf("second edit status = %d, body=%s", resp.Code, resp.Body.String())
	}
	second, err := s.historyStore.Current("codex")
	if err != nil {
		t.Fatal(err)
	}
	if second.Revision == first.Revision {
		t.Fatalf("second edit did not create a new revision: %#v", second)
	}

	// Corrupt the HEAD pointer file itself (not a revision file it points
	// at) with garbage that cannot resolve to any revision id.
	headPath := filepath.Join(historyRoot, "codex", "HEAD")
	if err := os.WriteFile(headPath, []byte("not-a-valid-revision-id"), 0o600); err != nil {
		t.Fatal(err)
	}

	if _, err := s.reloadProviderRegistry(); err != nil {
		t.Fatal(err)
	}

	// The registry must keep serving codex — with the override dropped,
	// not with a 5xx or a missing provider — while it is unresolvable.
	detailReq := httptest.NewRequest(http.MethodGet, "/api/providers/codex?token=test-token", nil)
	detailReq.Header.Set("Origin", "http://127.0.0.1:47777")
	detailReq.Host = "127.0.0.1:47777"
	detailResp := httptest.NewRecorder()
	s.handleProviderRoute(detailResp, detailReq)
	if detailResp.Code != http.StatusOK {
		t.Fatalf("GET codex after HEAD corruption status = %d, body=%s", detailResp.Code, detailResp.Body.String())
	}
	var detailBody struct {
		Provider provider.EffectiveDefinition `json:"provider"`
	}
	if err := json.Unmarshal(detailResp.Body.Bytes(), &detailBody); err != nil {
		t.Fatal(err)
	}
	if detailBody.Provider.EffectiveSource.Origin == provider.OriginOverride {
		t.Fatalf("codex still reports an override after its HEAD was corrupted: %#v", detailBody.Provider.EffectiveSource)
	}
	if detailBody.Provider.DisplayName == "Codex second edit" {
		t.Fatalf("codex kept serving the broken override's value: %#v", detailBody.Provider)
	}

	getResp := providerRecoveryGetRequest(t, s, "codex")
	if getResp.Code != http.StatusOK {
		t.Fatalf("GET recovery status = %d, body=%s", getResp.Code, getResp.Body.String())
	}
	var getBody struct {
		ProviderID string                      `json:"provider_id"`
		State      string                      `json:"state"`
		Candidates []providerRecoveryCandidate `json:"candidates"`
	}
	if err := json.Unmarshal(getResp.Body.Bytes(), &getBody); err != nil {
		t.Fatal(err)
	}
	if getBody.ProviderID != "codex" || getBody.State != "recovery_required" {
		t.Fatalf("recovery body = %#v, want provider_id=codex state=recovery_required", getBody)
	}
	if len(getBody.Candidates) != 2 {
		t.Fatalf("candidates = %#v, want 2 entries", getBody.Candidates)
	}
	if getBody.Candidates[0].Kind != "user_revision" || getBody.Candidates[0].Revision != second.Revision {
		t.Fatalf("candidates[0] = %#v, want the last readable revision %q", getBody.Candidates[0], second.Revision)
	}
	if getBody.Candidates[1].Kind != "base" || getBody.Candidates[1].Revision != "" {
		t.Fatalf("candidates[1] = %#v, want a bare base entry", getBody.Candidates[1])
	}

	// Recover to the *first* edit's revision, not the top candidate, to
	// prove a caller can pick any surviving revision rather than only the
	// most recent one.
	postResp := providerRecoveryPostRequest(t, s, "codex", first.Revision)
	if postResp.Code != http.StatusOK {
		t.Fatalf("POST recovery status = %d, body=%s", postResp.Code, postResp.Body.String())
	}
	var postBody struct {
		OK       bool   `json:"ok"`
		Revision string `json:"revision"`
	}
	if err := json.Unmarshal(postResp.Body.Bytes(), &postBody); err != nil {
		t.Fatal(err)
	}
	if !postBody.OK || postBody.Revision == "" {
		t.Fatalf("POST recovery body = %#v, want ok=true with a fresh revision", postBody)
	}

	afterReq := httptest.NewRequest(http.MethodGet, "/api/providers/codex?token=test-token", nil)
	afterReq.Header.Set("Origin", "http://127.0.0.1:47777")
	afterReq.Host = "127.0.0.1:47777"
	afterResp := httptest.NewRecorder()
	s.handleProviderRoute(afterResp, afterReq)
	if afterResp.Code != http.StatusOK {
		t.Fatalf("GET codex after recovery status = %d, body=%s", afterResp.Code, afterResp.Body.String())
	}
	var afterBody struct {
		Provider provider.EffectiveDefinition `json:"provider"`
	}
	if err := json.Unmarshal(afterResp.Body.Bytes(), &afterBody); err != nil {
		t.Fatal(err)
	}
	if afterBody.Provider.DisplayName != "Codex first edit" {
		t.Fatalf("codex was not recovered to the chosen revision: %#v", afterBody.Provider)
	}
	if afterBody.Provider.EffectiveSource.Origin != provider.OriginOverride {
		t.Fatalf("codex is not reported as an override after recovery: %#v", afterBody.Provider.EffectiveSource)
	}

	afterGetResp := providerRecoveryGetRequest(t, s, "codex")
	if afterGetResp.Code != http.StatusOK {
		t.Fatalf("GET recovery after POST status = %d, body=%s", afterGetResp.Code, afterGetResp.Body.String())
	}
	var afterGetBody struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(afterGetResp.Body.Bytes(), &afterGetBody); err != nil {
		t.Fatal(err)
	}
	if afterGetBody.State != "ok" {
		t.Fatalf("state after recovery = %q, want ok, body=%s", afterGetBody.State, afterGetResp.Body.String())
	}

	quarantineEntries, err := os.ReadDir(filepath.Join(backupRoot, "quarantine"))
	if err != nil {
		t.Fatalf("read quarantine dir: %v", err)
	}
	if len(quarantineEntries) < 1 {
		t.Fatalf("quarantine dir = %#v, want at least one quarantined HEAD", quarantineEntries)
	}
}

// TestProviderRollbackDrillCustomDeleteRestore is C12 C7's scenario 2: it
// creates a user-added (non-built-in) provider through the Hub's HTTP
// surface, deletes it (which HistoryStore.BackupSnapshot captures as a
// "before-delete" backup, same as TestProviderCreatePatchDeleteAPIUsesRevision
// already pins), and checks whether POST
// /api/providers/{id}/backups/{backupID}/restore can actually bring a
// deleted custom provider back — not just verify the backup content.
func TestProviderRollbackDrillCustomDeleteRestore(t *testing.T) {
	s := newProviderAPITestServer(t)
	const id = "rollback-drill-cli"
	definition := provider.Definition{
		SchemaVersion: 1,
		ID:            id,
		DisplayName:   "Rollback Drill CLI",
		Launch:        &provider.LaunchDefinition{Executable: "rollback-drill-cli"},
	}

	createReq := httptest.NewRequest(http.MethodPost, "/api/providers?token=test-token", bytes.NewReader(mustJSON(t, definition)))
	createReq.Header.Set("Origin", "http://127.0.0.1:47777")
	createReq.Host = "127.0.0.1:47777"
	createResp := httptest.NewRecorder()
	s.handleProviders(createResp, createReq)
	if createResp.Code != http.StatusCreated {
		t.Fatalf("POST /api/providers status = %d, body=%s", createResp.Code, createResp.Body.String())
	}

	registryAfterCreate := s.providerRegistrySnapshot()
	if _, ok := registryAfterCreate.Lookup(id); !ok {
		t.Fatal("created provider was not reloaded")
	}

	deleteReq := httptest.NewRequest(http.MethodDelete, "/api/providers/"+id+"?token=test-token&expected_revision="+registryAfterCreate.Revision(), nil)
	deleteReq.Header.Set("Origin", "http://127.0.0.1:47777")
	deleteReq.Host = "127.0.0.1:47777"
	deleteResp := httptest.NewRecorder()
	s.handleProviderRoute(deleteResp, deleteReq)
	if deleteResp.Code != http.StatusOK {
		t.Fatalf("DELETE provider status = %d, body=%s", deleteResp.Code, deleteResp.Body.String())
	}
	if _, ok := s.providerRegistrySnapshot().Lookup(id); ok {
		t.Fatal("deleted provider is still registered")
	}

	backups, err := s.historyStore.ListBackups(id)
	if err != nil {
		t.Fatal(err)
	}
	var beforeDelete provider.RevisionRecord
	found := false
	for _, backup := range backups {
		if backup.Reason == "before-delete" {
			beforeDelete = backup
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("no before-delete backup among %#v", backups)
	}

	restoreBody := map[string]any{"expected_revision": ""}
	restoreReq := httptest.NewRequest(http.MethodPost, "/api/providers/"+id+"/backups/"+beforeDelete.Revision+"/restore?token=test-token", bytes.NewReader(mustJSON(t, restoreBody)))
	restoreReq.Header.Set("Origin", "http://127.0.0.1:47777")
	restoreReq.Host = "127.0.0.1:47777"
	restoreResp := httptest.NewRecorder()
	s.handleProviderRoute(restoreResp, restoreReq)
	if restoreResp.Code != http.StatusOK {
		t.Fatalf("POST backup restore after delete status = %d, body=%s", restoreResp.Code, restoreResp.Body.String())
	}

	restored, ok := s.providerRegistrySnapshot().Lookup(id)
	if !ok || restored.DisplayName != "Rollback Drill CLI" {
		t.Fatalf("deleted custom provider was not restored from its before-delete backup: ok=%v, %#v", ok, restored)
	}
}

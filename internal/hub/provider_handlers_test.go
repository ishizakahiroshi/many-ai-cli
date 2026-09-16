package hub

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"sync"
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
	history, err := provider.NewHistoryStore(filepath.Join(t.TempDir(), "history.d"), filepath.Join(t.TempDir(), "backups.d"))
	if err != nil {
		t.Fatal(err)
	}
	s.historyStore = history
	registry, diagnostics, err := buildProviderRegistry(s.cfg)
	if err != nil || len(diagnostics) != 0 {
		t.Fatalf("build provider registry: %v, %#v", err, diagnostics)
	}
	s.providers = registry
	return s
}

func patchProviderRequest(t *testing.T, s *Server, id, expectedRevision string, definition provider.Definition) *httptest.ResponseRecorder {
	t.Helper()
	body := map[string]any{"expected_revision": expectedRevision, "definition": definition}
	req := httptest.NewRequest(http.MethodPatch, "/api/providers/"+id+"?token=test-token", bytes.NewReader(mustJSON(t, body)))
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderRoute(resp, req)
	return resp
}

func deleteProviderRequest(t *testing.T, s *Server, id, expectedRevision string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodDelete, "/api/providers/"+id+"?token=test-token&expected_revision="+expectedRevision, nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderRoute(resp, req)
	return resp
}

// TestBuiltinProviderFirstOverrideSucceeds pins the C1 fix: a built-in
// provider that has never been overridden has an empty HistoryStore parent
// revision, which is a different concept from the whole-registry Revision().
// Before the fix, the handler always compared expected_revision against
// registry.Revision(), so the very first PATCH of any built-in provider was
// rejected as a stale conflict no client could ever satisfy.
func TestBuiltinProviderFirstOverrideSucceeds(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	definition.Definition.DisplayName = "Claude (edited)"

	resp := patchProviderRequest(t, s, "claude", "", definition.Definition)
	if resp.Code != http.StatusOK {
		t.Fatalf("first builtin PATCH status = %d, body=%s", resp.Code, resp.Body.String())
	}

	updated, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok || updated.Definition.DisplayName != "Claude (edited)" {
		t.Fatalf("override was not applied: %#v", updated)
	}
	if updated.EffectiveSource.Origin != provider.OriginOverride || updated.EffectiveSource.Revision == "" {
		t.Fatalf("effective source after override = %#v, want non-empty override revision", updated.EffectiveSource)
	}
}

func TestBuiltinProviderSecondOverrideAndStaleConflict(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition, _ := s.providerRegistrySnapshot().Lookup("claude")
	definition.Definition.DisplayName = "Claude v1"
	first := patchProviderRequest(t, s, "claude", "", definition.Definition)
	if first.Code != http.StatusOK {
		t.Fatalf("first PATCH status = %d, body=%s", first.Code, first.Body.String())
	}
	afterFirst, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	currentRevision := afterFirst.EffectiveSource.Revision

	definition.Definition.DisplayName = "Claude v2"
	second := patchProviderRequest(t, s, "claude", currentRevision, definition.Definition)
	if second.Code != http.StatusOK {
		t.Fatalf("second PATCH status = %d, body=%s", second.Code, second.Body.String())
	}

	// Replaying the first (now-stale) expected revision must conflict, not
	// silently reuse the old parent.
	stale := patchProviderRequest(t, s, "claude", "", definition.Definition)
	if stale.Code != http.StatusConflict {
		t.Fatalf("stale builtin PATCH status = %d, want 409, body=%s", stale.Code, stale.Body.String())
	}
	var errBody struct {
		Error string `json:"error"`
	}
	if err := json.Unmarshal(stale.Body.Bytes(), &errBody); err != nil {
		t.Fatal(err)
	}
	if errBody.Error != "revision_conflict" {
		t.Fatalf("stale conflict code = %q, want revision_conflict", errBody.Error)
	}
}

func TestBuiltinProviderDisableAndRestore(t *testing.T) {
	s := newProviderAPITestServer(t)

	// First establish an enabled, edited revision (A) so there is something
	// meaningful to restore back to after disabling.
	edited, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	edited.Definition.DisplayName = "Claude (edited)"
	editResp := patchProviderRequest(t, s, "claude", "", edited.Definition)
	if editResp.Code != http.StatusOK {
		t.Fatalf("edit before disable status = %d, body=%s", editResp.Code, editResp.Body.String())
	}
	afterEdit, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	editedRevision := afterEdit.EffectiveSource.Revision

	disableResp := deleteProviderRequest(t, s, "claude", editedRevision)
	if disableResp.Code != http.StatusOK {
		t.Fatalf("builtin disable status = %d, body=%s", disableResp.Code, disableResp.Body.String())
	}
	disabled, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok || disabled.Enabled == nil || *disabled.Enabled {
		t.Fatalf("claude was not disabled: %#v", disabled)
	}

	// Replaying the pre-disable revision as "expected" is now stale.
	staleDisable := deleteProviderRequest(t, s, "claude", editedRevision)
	if staleDisable.Code != http.StatusConflict {
		t.Fatalf("repeat disable with stale revision status = %d, want 409, body=%s", staleDisable.Code, staleDisable.Body.String())
	}

	restoreBody := map[string]any{"revision": editedRevision, "expected_revision": disabled.EffectiveSource.Revision}
	restoreReq := httptest.NewRequest(http.MethodPost, "/api/providers/claude/restore?token=test-token", bytes.NewReader(mustJSON(t, restoreBody)))
	restoreReq.Header.Set("Origin", "http://127.0.0.1:47777")
	restoreReq.Host = "127.0.0.1:47777"
	restoreResp := httptest.NewRecorder()
	s.handleProviderRoute(restoreResp, restoreReq)
	if restoreResp.Code != http.StatusOK {
		t.Fatalf("restore status = %d, body=%s", restoreResp.Code, restoreResp.Body.String())
	}
	restored, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok || restored.Enabled == nil || !*restored.Enabled || restored.DisplayName != "Claude (edited)" {
		t.Fatalf("claude was not restored to the pre-disable revision: %#v", restored)
	}
}

// TestBuiltinProviderDisableSavesSparseOverride pins the C5 fix at the HTTP
// layer: disabling a never-before-edited builtin provider must not freeze
// its launch/model/adapter fields into the override. Before this fix,
// handleProviderDelete built the override payload from registry.Lookup's
// fully merged EffectiveDefinition, so a disable action captured every
// field regardless of whether the user had ever touched it.
func TestBuiltinProviderDisableSavesSparseOverride(t *testing.T) {
	s := newProviderAPITestServer(t)
	resp := deleteProviderRequest(t, s, "claude", "")
	if resp.Code != http.StatusOK {
		t.Fatalf("disable status = %d, body=%s", resp.Code, resp.Body.String())
	}
	revisions, err := s.historyStore.List("claude")
	if err != nil || len(revisions) != 1 {
		t.Fatalf("history list = %#v, %v", revisions, err)
	}
	payload := revisions[0].Payload
	if payload.Launch != nil {
		t.Fatalf("disable captured launch into the override: %#v", payload.Launch)
	}
	if payload.DisplayName != "" {
		t.Fatalf("disable captured display_name into the override: %q", payload.DisplayName)
	}
	if payload.Enabled == nil || *payload.Enabled {
		t.Fatalf("disable did not persist enabled:false: %#v", payload.Enabled)
	}
	// The registry's resolved view must still show the real embedded
	// launch/display_name, proving they are still coming from the embedded
	// layer rather than from a frozen copy in the override.
	definition, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok || definition.Launch == nil || definition.Launch.Executable == "" {
		t.Fatalf("claude lost its embedded launch definition after a sparse disable: %#v", definition)
	}
}

func TestProviderCreateConcurrentSameIDRejectsLoser(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition := provider.Definition{SchemaVersion: 1, ID: "race-cli", DisplayName: "Race CLI", Launch: &provider.LaunchDefinition{Executable: "race-cli"}}
	const attempts = 6
	codes := make([]int, attempts)
	var wg sync.WaitGroup
	for i := 0; i < attempts; i++ {
		wg.Add(1)
		go func(index int) {
			defer wg.Done()
			req := httptest.NewRequest(http.MethodPost, "/api/providers?token=test-token", bytes.NewReader(mustJSON(t, definition)))
			req.Header.Set("Origin", "http://127.0.0.1:47777")
			req.Host = "127.0.0.1:47777"
			resp := httptest.NewRecorder()
			s.handleProviders(resp, req)
			codes[index] = resp.Code
		}(i)
	}
	wg.Wait()
	created := 0
	for _, code := range codes {
		switch code {
		case http.StatusCreated:
			created++
		case http.StatusConflict:
			// expected for every loser
		default:
			t.Fatalf("unexpected concurrent create status: %d", code)
		}
	}
	if created != 1 {
		t.Fatalf("created count = %d, want exactly 1 (codes=%v)", created, codes)
	}
}

// TestProviderListReportsCommandMissingDiagnostic pins the C2 fix: PATH
// membership is not part of a Definition, so it cannot come from
// registry.Diagnostics() (a pure function of the merged definitions). The
// list and detail handlers must add it as a live, request-time diagnostic so
// the UI can tell "the definition is fine but the CLI isn't installed" apart
// from a plain "available" row.
func TestProviderListReportsCommandMissingDiagnostic(t *testing.T) {
	s := newProviderAPITestServer(t)
	original := providerCommandLookPath
	defer func() { providerCommandLookPath = original }()
	providerCommandLookPath = func(file string) (string, error) {
		if file == "claude" {
			return "", fmt.Errorf("not found")
		}
		return "/usr/bin/" + file, nil
	}

	req := httptest.NewRequest(http.MethodGet, "/api/providers?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviders(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("GET /api/providers status = %d", resp.Code)
	}
	var body struct {
		Diagnostics []provider.Diagnostic `json:"diagnostics"`
	}
	if err := json.Unmarshal(resp.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, diagnostic := range body.Diagnostics {
		if diagnostic.Code == "command_missing" && diagnostic.Field == "claude" {
			found = true
		}
		if diagnostic.Field == "codex" && diagnostic.Code == "command_missing" {
			t.Fatalf("codex should not be flagged as command_missing: %#v", diagnostic)
		}
	}
	if !found {
		t.Fatalf("expected a command_missing diagnostic for claude, got %#v", body.Diagnostics)
	}

	detailReq := httptest.NewRequest(http.MethodGet, "/api/providers/claude?token=test-token", nil)
	detailReq.Header.Set("Origin", "http://127.0.0.1:47777")
	detailReq.Host = "127.0.0.1:47777"
	detailResp := httptest.NewRecorder()
	s.handleProviderRoute(detailResp, detailReq)
	var detailBody struct {
		Diagnostics []provider.Diagnostic `json:"diagnostics"`
	}
	if err := json.Unmarshal(detailResp.Body.Bytes(), &detailBody); err != nil {
		t.Fatal(err)
	}
	if len(detailBody.Diagnostics) != 1 || detailBody.Diagnostics[0].Code != "command_missing" {
		t.Fatalf("claude detail diagnostics = %#v, want exactly one command_missing", detailBody.Diagnostics)
	}
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

// TestProviderBackupAPIListsVerifiesAndRestores pins the C5 fix: the
// `many-ai-cli provider backup` CLI already used
// HistoryStore.ListBackups/VerifyBackup/RestoreBackup, but the HTTP API had
// no endpoint for any of it, only for the separate revisions list. The UI
// and CLI must be able to reach the same verified-backup restore path.
func TestProviderBackupAPIListsVerifiesAndRestores(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	definition.Definition.DisplayName = "Claude v1"
	if resp := patchProviderRequest(t, s, "claude", "", definition.Definition); resp.Code != http.StatusOK {
		t.Fatalf("first edit status = %d, body=%s", resp.Code, resp.Body.String())
	}
	afterFirst, _ := s.providerRegistrySnapshot().Lookup("claude")
	definition.Definition.DisplayName = "Claude v2"
	if resp := patchProviderRequest(t, s, "claude", afterFirst.EffectiveSource.Revision, definition.Definition); resp.Code != http.StatusOK {
		t.Fatalf("second edit status = %d, body=%s", resp.Code, resp.Body.String())
	}

	listReq := httptest.NewRequest(http.MethodGet, "/api/providers/claude/backups?token=test-token", nil)
	listReq.Header.Set("Origin", "http://127.0.0.1:47777")
	listReq.Host = "127.0.0.1:47777"
	listResp := httptest.NewRecorder()
	s.handleProviderRoute(listResp, listReq)
	if listResp.Code != http.StatusOK {
		t.Fatalf("list backups status = %d, body=%s", listResp.Code, listResp.Body.String())
	}
	var listBody struct {
		Backups []provider.RevisionRecord `json:"backups"`
	}
	if err := json.Unmarshal(listResp.Body.Bytes(), &listBody); err != nil {
		t.Fatal(err)
	}
	if len(listBody.Backups) == 0 {
		t.Fatalf("expected at least one backup after two edits, got %#v", listBody.Backups)
	}
	backupID := listBody.Backups[0].Revision

	verifyReq := httptest.NewRequest(http.MethodGet, "/api/providers/claude/backups/"+backupID+"/verify?token=test-token", nil)
	verifyReq.Header.Set("Origin", "http://127.0.0.1:47777")
	verifyReq.Host = "127.0.0.1:47777"
	verifyResp := httptest.NewRecorder()
	s.handleProviderRoute(verifyResp, verifyReq)
	if verifyResp.Code != http.StatusOK {
		t.Fatalf("verify backup status = %d, body=%s", verifyResp.Code, verifyResp.Body.String())
	}

	currentRevision, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	restoreBody := map[string]any{"expected_revision": currentRevision.EffectiveSource.Revision}
	restoreReq := httptest.NewRequest(http.MethodPost, "/api/providers/claude/backups/"+backupID+"/restore?token=test-token", bytes.NewReader(mustJSON(t, restoreBody)))
	restoreReq.Header.Set("Origin", "http://127.0.0.1:47777")
	restoreReq.Host = "127.0.0.1:47777"
	restoreResp := httptest.NewRecorder()
	s.handleProviderRoute(restoreResp, restoreReq)
	if restoreResp.Code != http.StatusOK {
		t.Fatalf("restore backup status = %d, body=%s", restoreResp.Code, restoreResp.Body.String())
	}
	restored, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok || restored.DisplayName != "Claude v1" {
		t.Fatalf("claude was not restored from the verified backup: %#v", restored)
	}
}

func TestProviderBackupVerifyRejectsUnknownBackup(t *testing.T) {
	s := newProviderAPITestServer(t)
	definition, _ := s.providerRegistrySnapshot().Lookup("claude")
	definition.Definition.DisplayName = "Claude v1"
	patchProviderRequest(t, s, "claude", "", definition.Definition)

	req := httptest.NewRequest(http.MethodGet, "/api/providers/claude/backups/does-not-exist/verify?token=test-token", nil)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderRoute(resp, req)
	if resp.Code != http.StatusUnprocessableEntity {
		t.Fatalf("verify unknown backup status = %d, want 422, body=%s", resp.Code, resp.Body.String())
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

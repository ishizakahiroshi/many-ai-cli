package hub

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os/exec"
	"strings"

	"many-ai-cli/internal/provider"
)

// providerCommandLookPath is a seam so handler tests can force a "not found"
// result without depending on what happens to be on the test machine's PATH.
var providerCommandLookPath = exec.LookPath

// providerCommandDiagnostics reports, per provider, whether its launch
// executable can be found on PATH. This is a live, request-time check (PATH
// membership is not part of a Definition and is not captured by
// ValidateDefinition), so the UI's "available / disabled / definition error /
// command missing" row status previously had nothing to distinguish "the
// definition is fine but the CLI isn't installed" from "available".
func providerCommandDiagnostics(registry *provider.Registry) []provider.Diagnostic {
	if registry == nil {
		return nil
	}
	var diagnostics []provider.Diagnostic
	for _, summary := range registry.List() {
		definition, ok := registry.Lookup(summary.ID)
		if !ok || definition.Launch == nil {
			continue
		}
		if providerCommandFound(definition.Launch) {
			continue
		}
		diagnostics = append(diagnostics, provider.Diagnostic{
			Code:     "command_missing",
			Severity: provider.SeverityWarning,
			Field:    summary.ID,
			Message:  fmt.Sprintf("executable for %q was not found on PATH", summary.ID),
		})
	}
	return diagnostics
}

func providerCommandFound(launch *provider.LaunchDefinition) bool {
	var candidates []string
	if launch.Executable != "" {
		candidates = append(candidates, launch.Executable)
	}
	candidates = append(candidates, launch.ExecutableCandidates...)
	if len(candidates) == 0 {
		// No candidate to check is a schema problem the validator already
		// flags elsewhere; don't also claim the command is missing.
		return true
	}
	for _, candidate := range candidates {
		if candidate == "" {
			continue
		}
		if _, err := providerCommandLookPath(candidate); err == nil {
			return true
		}
	}
	return false
}

// writeProviderHistoryError maps a HistoryStore write failure to its HTTP
// status: a stale optimistic-lock revision is 409 (the client should reload
// and retry), everything else (invalid payload, missing store, etc.) is the
// caller-supplied fallback. Without this split, the built-in provider's very
// first override always came back as a generic failure the UI could not tell
// apart from an actual conflict.
//
// An edit that would empty a field the distributed definition fills in is
// 422 provider_override_clears_value with the field names in "fields", so the
// dialog can name them in the user's language. It stays 422, not 409: the UI
// reads 409 as "changed elsewhere, reload", which is not what happened.
func writeProviderHistoryError(w http.ResponseWriter, fallbackCode string, err error) {
	if errors.Is(err, provider.ErrRevisionConflict) {
		writeJSONError(w, http.StatusConflict, "revision_conflict", err.Error())
		return
	}
	var clears *provider.OverrideClearsValueError
	if errors.As(err, &clears) {
		writeJSONStatus(w, http.StatusUnprocessableEntity, map[string]any{
			"ok":     false,
			"error":  "provider_override_clears_value",
			"detail": err.Error(),
			"fields": clears.Fields,
		})
		return
	}
	writeJSONError(w, http.StatusUnprocessableEntity, fallbackCode, err.Error())
}

type providerListResponse struct {
	Revision    string                `json:"revision"`
	Providers   []provider.Summary    `json:"providers"`
	Diagnostics []provider.Diagnostic `json:"diagnostics,omitempty"`
}

type providerDetailResponse struct {
	Revision    string                       `json:"revision"`
	Provider    provider.EffectiveDefinition `json:"provider"`
	Diagnostics []provider.Diagnostic        `json:"diagnostics,omitempty"`
}

func (s *Server) handleProviders(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodPost {
		s.handleProviderCreate(w, r)
		return
	}
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, providerListResponse{
		Revision:    registry.Revision(),
		Providers:   registry.List(),
		Diagnostics: append(registry.Diagnostics(), providerCommandDiagnostics(registry)...),
	})
}

func (s *Server) handleProviderCreate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.providerStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_store_unavailable", "provider store is unavailable")
		return
	}
	var definition provider.Definition
	if !decodeJSON(w, r, &definition) {
		return
	}
	if provider.IsBuiltinID(definition.ID) || definition.ID == "shell" {
		writeJSONError(w, http.StatusConflict, "provider_id_reserved", "provider id is reserved")
		return
	}
	// CreateNew is the atomic create: two concurrent POSTs for the same id
	// cannot both succeed. A prior Lookup-then-Save here raced two requests
	// through the same in-memory "not found yet" window and let the second
	// writer silently overwrite the first with both getting 201 Created.
	if err := s.providerStore.CreateNew(definition); err != nil {
		if errors.Is(err, provider.ErrProviderAlreadyExists) {
			writeJSONError(w, http.StatusConflict, "provider_id_exists", "provider id already exists")
			return
		}
		writeJSONError(w, http.StatusUnprocessableEntity, "provider_save_failed", err.Error())
		return
	}
	if s.historyStore != nil {
		if _, err := s.historyStore.BackupSnapshot(definition.ID, definition, "create"); err != nil {
			_ = s.providerStore.Delete(definition.ID)
			writeJSONError(w, http.StatusInternalServerError, "provider_backup_failed", err.Error())
			return
		}
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSONStatus(w, http.StatusCreated, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

func (s *Server) handleProviderRoute(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/api/providers/")
	if path == "validate" {
		s.handleProviderValidate(w, r)
		return
	}
	parts := strings.Split(path, "/")
	if len(parts) == 2 && parts[1] == "history" {
		s.handleProviderHistory(w, r, parts[0])
		return
	}
	if len(parts) == 4 && parts[1] == "history" && parts[3] == "diff" {
		s.handleProviderHistoryDiff(w, r, parts[0], parts[2])
		return
	}
	if len(parts) == 2 && parts[1] == "reset" {
		s.handleProviderReset(w, r, parts[0])
		return
	}
	if len(parts) == 2 && parts[1] == "restore" {
		s.handleProviderRestore(w, r, parts[0])
		return
	}
	if len(parts) == 2 && parts[1] == "backups" {
		s.handleProviderBackups(w, r, parts[0])
		return
	}
	if len(parts) == 4 && parts[1] == "backups" && parts[3] == "verify" {
		s.handleProviderBackupVerify(w, r, parts[0], parts[2])
		return
	}
	if len(parts) == 4 && parts[1] == "backups" && parts[3] == "restore" {
		s.handleProviderBackupRestore(w, r, parts[0], parts[2])
		return
	}
	if len(parts) == 2 && parts[1] == "recovery" {
		s.handleProviderRecovery(w, r, parts[0])
		return
	}
	if path == "" {
		if !s.guard(w, r, r.Method) {
			return
		}
		writeJSONError(w, http.StatusNotFound, "not_found", "provider endpoint not found")
		return
	}
	if strings.Contains(path, "/") {
		writeJSONError(w, http.StatusNotFound, "not_found", "provider endpoint not found")
		return
	}
	if r.Method == http.MethodGet {
		s.handleProviderDetail(w, r, path)
		return
	}
	if r.Method == http.MethodPatch {
		s.handleProviderPatch(w, r, path)
		return
	}
	if r.Method == http.MethodDelete {
		s.handleProviderDelete(w, r, path)
		return
	}
	writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
}

func (s *Server) handleProviderDetail(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	definition, ok := registry.Lookup(id)
	if !ok {
		writeJSONError(w, http.StatusNotFound, "provider_not_found", "provider was not found")
		return
	}
	var diagnostics []provider.Diagnostic
	for _, diagnostic := range append(registry.Diagnostics(), providerCommandDiagnostics(registry)...) {
		if diagnostic.Field == id || strings.HasPrefix(diagnostic.Field, id+".") {
			diagnostics = append(diagnostics, diagnostic)
		}
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, providerDetailResponse{Revision: registry.Revision(), Provider: definition, Diagnostics: diagnostics})
}

func (s *Server) handleProviderValidate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var raw json.RawMessage
	if !decodeJSON(w, r, &raw) {
		return
	}
	diagnostics, err := provider.ValidateDefinition(raw, provider.DefaultAdapterCatalog())
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_provider_definition", err.Error())
		return
	}
	valid := true
	for _, diagnostic := range diagnostics {
		if diagnostic.IsError() {
			valid = false
			break
		}
	}
	writeJSON(w, map[string]any{"valid": valid, "diagnostics": diagnostics})
}

func (s *Server) handleProviderPatch(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodPatch) {
		return
	}
	if s.providerStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_store_unavailable", "provider store is unavailable")
		return
	}
	var body struct {
		ExpectedRevision *string             `json:"expected_revision"`
		Definition       provider.Definition `json:"definition"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	// ExpectedRevision is a pointer so a JSON body can express "" (a built-in
	// provider that has never been overridden yet, so its history has no
	// parent revision) as distinct from the field being missing entirely.
	// A plain string could not tell those apart, which is why the first edit
	// of any built-in provider used to be rejected unconditionally below.
	if body.ExpectedRevision == nil {
		writeJSONError(w, http.StatusBadRequest, "expected_revision_required", "expected_revision is required")
		return
	}
	expected := *body.ExpectedRevision
	if body.Definition.ID == "" {
		body.Definition.ID = id
	}
	if body.Definition.ID != id {
		writeJSONError(w, http.StatusBadRequest, "provider_id_mismatch", "provider id cannot change")
		return
	}
	if provider.IsBuiltinID(id) {
		// Built-in providers are versioned through HistoryStore, whose
		// revision is per-provider (empty until the first override exists).
		// That is a different value from the whole-registry Revision() used
		// below for the custom-provider store, so it must not be checked
		// against registry.Revision() here — SaveOverride does its own
		// per-provider optimistic-lock check against the real current parent.
		if s.historyStore == nil {
			writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
			return
		}
		if _, err := s.historyStore.SaveEffectiveOverride(id, body.Definition, s.baselineProviderDefinition(id), expected, "edit"); err != nil {
			writeProviderHistoryError(w, "provider_override_failed", err)
			return
		}
		if diagnostics, err := s.reloadProviderRegistry(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
			return
		} else {
			writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
		}
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil || registry.Revision() != expected {
		writeJSONError(w, http.StatusConflict, "revision_conflict", "provider registry changed; reload and try again")
		return
	}
	if current, ok := registry.Lookup(id); ok && s.historyStore != nil {
		if _, err := s.historyStore.BackupSnapshot(id, current.Definition, "before-edit"); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_backup_failed", err.Error())
			return
		}
	}
	if err := s.providerStore.Save(body.Definition); err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "provider_save_failed", err.Error())
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

func (s *Server) handleProviderDelete(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodDelete) {
		return
	}
	if s.providerStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_store_unavailable", "provider store is unavailable")
		return
	}
	// Query.Has, not Get()+empty check, so a built-in provider's legitimate
	// "no history yet" expectation (an explicit empty string) is not
	// confused with the parameter being omitted altogether.
	query := r.URL.Query()
	if !query.Has("expected_revision") {
		writeJSONError(w, http.StatusBadRequest, "expected_revision_required", "expected_revision is required")
		return
	}
	expected := query.Get("expected_revision")
	if provider.IsBuiltinID(id) {
		// See handleProviderPatch: built-in providers are gated by their own
		// HistoryStore revision, not registry.Revision(), so there is no
		// registry-wide check here before SaveOverride does its own.
		if s.historyStore == nil {
			writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
			return
		}
		registry := s.providerRegistrySnapshot()
		if registry == nil {
			writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
			return
		}
		if _, ok := registry.Lookup(id); !ok {
			writeJSONError(w, http.StatusNotFound, "provider_not_found", "provider was not found")
			return
		}
		// A sparse {enabled: false} override — not the full resolved
		// definition — so disabling a provider that was never edited before
		// does not also freeze its launch/model/adapter fields at whatever
		// they happened to resolve to today. SaveOverride merges this onto
		// whatever is already overridden (nothing, the first time) and
		// validates the result against the current embedded/distribution
		// baseline, so it still comes out schema-valid.
		disabled := false
		if _, err := s.historyStore.SaveOverride(id, provider.Definition{ID: id, Enabled: &disabled}, s.baselineProviderDefinition(id), expected, "delete"); err != nil {
			writeProviderHistoryError(w, "provider_override_failed", err)
			return
		}
		if diagnostics, err := s.reloadProviderRegistry(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
			return
		} else {
			writeJSON(w, map[string]any{"ok": true, "disabled": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
		}
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil || registry.Revision() != expected {
		writeJSONError(w, http.StatusConflict, "revision_conflict", "provider registry changed; reload and try again")
		return
	}
	if current, ok := registry.Lookup(id); ok && s.historyStore != nil {
		if _, err := s.historyStore.BackupSnapshot(id, current.Definition, "before-delete"); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_backup_failed", err.Error())
			return
		}
	}
	if err := s.providerStore.Delete(id); err != nil {
		writeJSONError(w, http.StatusNotFound, "provider_delete_failed", err.Error())
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "removed": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

func (s *Server) handleProviderHistory(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	revisions, err := s.historyStore.List(id)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "history_not_found", err.Error())
		return
	}
	writeJSON(w, map[string]any{"provider_id": id, "revisions": revisions})
}

func (s *Server) handleProviderHistoryDiff(w http.ResponseWriter, r *http.Request, id, revision string) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	selected, err := s.historyStore.GetRevision(id, revision)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "history_not_found", err.Error())
		return
	}
	current, err := s.historyStore.Current(id)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "history_not_found", err.Error())
		return
	}
	writeJSON(w, map[string]any{
		"provider_id":      id,
		"revision":         revision,
		"current_revision": current.Revision,
		"diff":             provider.DiffDistribution([]provider.Definition{current.Payload}, []provider.Definition{selected.Payload}, nil),
	})
}

func (s *Server) handleProviderReset(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if !provider.IsBuiltinID(id) {
		writeJSONError(w, http.StatusBadRequest, "provider_reset_not_builtin", "only built-in providers can be reset")
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	var body struct {
		ExpectedRevision *string `json:"expected_revision"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.ExpectedRevision == nil {
		writeJSONError(w, http.StatusBadRequest, "expected_revision_required", "expected_revision is required")
		return
	}
	if _, err := s.historyStore.Reset(id, *body.ExpectedRevision); err != nil {
		writeProviderHistoryError(w, "provider_reset_failed", err)
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

func (s *Server) handleProviderRestore(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	var body struct {
		Revision         string `json:"revision"`
		ExpectedRevision string `json:"expected_revision"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if strings.TrimSpace(body.Revision) == "" || strings.TrimSpace(body.ExpectedRevision) == "" {
		writeJSONError(w, http.StatusBadRequest, "revision_required", "revision and expected_revision are required")
		return
	}
	if _, err := s.historyStore.Restore(id, body.Revision, body.ExpectedRevision); err != nil {
		writeProviderHistoryError(w, "provider_restore_failed", err)
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

// handleProviderBackups, handleProviderBackupVerify and
// handleProviderBackupRestore expose the same HistoryStore backup service
// the `many-ai-cli provider backup` CLI already used (ListBackups /
// VerifyBackup / RestoreBackup) — before this, the CLI could inspect and
// restore a verified backup but the HTTP API (and therefore the UI) had no
// access to backups at all, only to the separate revisions list.
func (s *Server) handleProviderBackups(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	backups, err := s.historyStore.ListBackups(id)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "backups_not_found", err.Error())
		return
	}
	writeJSON(w, map[string]any{"provider_id": id, "backups": backups})
}

func (s *Server) handleProviderBackupVerify(w http.ResponseWriter, r *http.Request, id, backupID string) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	backup, err := s.historyStore.VerifyBackup(id, backupID)
	if err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "backup_verify_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true, "backup": backup})
}

func (s *Server) handleProviderBackupRestore(w http.ResponseWriter, r *http.Request, id, backupID string) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	var body struct {
		ExpectedRevision *string `json:"expected_revision"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.ExpectedRevision == nil {
		writeJSONError(w, http.StatusBadRequest, "expected_revision_required", "expected_revision is required")
		return
	}
	if _, err := s.historyStore.RestoreBackup(id, backupID, *body.ExpectedRevision); err != nil {
		writeProviderHistoryError(w, "backup_restore_failed", err)
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

// providerRecoveryCandidate is one option handleProviderRecoveryGet offers
// for repairing a provider whose HEAD cannot be resolved: either the most
// recently verified user revision (kind "user_revision", when one exists) or
// the override-free base (kind "base", always present when recovery is
// required). It intentionally does not distinguish an accepted distribution
// base from the embedded base — RecoverHead("", "") resolves that the same
// way Reset does, so the client does not need to know which one it is.
type providerRecoveryCandidate struct {
	Kind      string `json:"kind"`
	Revision  string `json:"revision,omitempty"`
	CreatedAt string `json:"created_at,omitempty"`
}

// handleProviderRecovery dispatches GET/POST for
// /api/providers/{id}/recovery to the same HistoryStore.NeedsRecovery /
// LastVerifiedRevision / RecoverHead trio C7/C8 added, so a provider whose
// HEAD cannot be resolved (LoadOverrides silently dropped its override) has
// a way for the UI and any other client to discover that and repair it.
func (s *Server) handleProviderRecovery(w http.ResponseWriter, r *http.Request, id string) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	if s.historyStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
		return
	}
	if r.Method == http.MethodPost {
		s.handleProviderRecoveryPost(w, r, id)
		return
	}
	s.handleProviderRecoveryGet(w, r, id)
}

func (s *Server) handleProviderRecoveryGet(w http.ResponseWriter, r *http.Request, id string) {
	needsRecovery, err := s.historyStore.NeedsRecovery(id)
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_provider_id", err.Error())
		return
	}
	state := "ok"
	// Always a non-nil (possibly empty) slice, never nil, so the "ok" state
	// serializes as [] instead of null.
	candidates := []providerRecoveryCandidate{}
	if needsRecovery {
		state = "recovery_required"
		if latest, found, lastErr := s.historyStore.LastVerifiedRevision(id); lastErr == nil && found {
			candidates = append(candidates, providerRecoveryCandidate{
				Kind:      "user_revision",
				Revision:  latest.Revision,
				CreatedAt: latest.CreatedAt,
			})
		}
		candidates = append(candidates, providerRecoveryCandidate{Kind: "base"})
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"provider_id": id,
		"state":       state,
		"candidates":  candidates,
	})
}

func (s *Server) handleProviderRecoveryPost(w http.ResponseWriter, r *http.Request, id string) {
	var body struct {
		Revision string `json:"revision"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	recovered, err := s.historyStore.RecoverHead(id, body.Revision)
	if err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "provider_recovery_failed", err.Error())
		return
	}
	if _, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true, "revision": recovered.Revision})
}

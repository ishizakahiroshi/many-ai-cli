package hub

import (
	"encoding/json"
	"net/http"
	"strings"

	"many-ai-cli/internal/provider"
)

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
		Diagnostics: registry.Diagnostics(),
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
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	if _, exists := registry.Lookup(definition.ID); exists {
		writeJSONError(w, http.StatusConflict, "provider_id_exists", "provider id already exists")
		return
	}
	if err := s.providerStore.Save(definition); err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "provider_save_failed", err.Error())
		return
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
	if len(parts) == 2 && parts[1] == "restore" {
		s.handleProviderRestore(w, r, parts[0])
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
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, providerDetailResponse{Revision: registry.Revision(), Provider: definition})
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
		ExpectedRevision string              `json:"expected_revision"`
		Definition       provider.Definition `json:"definition"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if strings.TrimSpace(body.ExpectedRevision) == "" {
		writeJSONError(w, http.StatusBadRequest, "expected_revision_required", "expected_revision is required")
		return
	}
	registry := s.providerRegistrySnapshot()
	if registry == nil || registry.Revision() != body.ExpectedRevision {
		writeJSONError(w, http.StatusConflict, "revision_conflict", "provider registry changed; reload and try again")
		return
	}
	if body.Definition.ID == "" {
		body.Definition.ID = id
	}
	if body.Definition.ID != id {
		writeJSONError(w, http.StatusBadRequest, "provider_id_mismatch", "provider id cannot change")
		return
	}
	if provider.IsBuiltinID(id) {
		if s.historyStore == nil {
			writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
			return
		}
		if _, err := s.historyStore.SaveOverride(id, body.Definition, body.ExpectedRevision, "edit"); err != nil {
			writeJSONError(w, http.StatusUnprocessableEntity, "provider_override_failed", err.Error())
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
	expected := strings.TrimSpace(r.URL.Query().Get("expected_revision"))
	registry := s.providerRegistrySnapshot()
	if expected == "" || registry == nil || registry.Revision() != expected {
		writeJSONError(w, http.StatusConflict, "revision_conflict", "provider registry changed; reload and try again")
		return
	}
	if provider.IsBuiltinID(id) {
		if s.historyStore == nil {
			writeJSONError(w, http.StatusServiceUnavailable, "history_store_unavailable", "provider history store is unavailable")
			return
		}
		disabled := false
		current, ok := s.providerRegistrySnapshot().Lookup(id)
		if !ok {
			writeJSONError(w, http.StatusNotFound, "provider_not_found", "provider was not found")
			return
		}
		current.Definition.Enabled = &disabled
		if _, err := s.historyStore.SaveOverride(id, current.Definition, expected, "delete"); err != nil {
			writeJSONError(w, http.StatusUnprocessableEntity, "provider_override_failed", err.Error())
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
		writeJSONError(w, http.StatusConflict, "provider_restore_failed", err.Error())
		return
	}
	if diagnostics, err := s.reloadProviderRegistry(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", err.Error())
		return
	} else {
		writeJSON(w, map[string]any{"ok": true, "revision": s.providerRegistrySnapshot().Revision(), "diagnostics": diagnostics})
	}
}

package hub

import (
	"crypto/ed25519"
	"errors"
	"net/http"
	"strings"

	"many-ai-cli/internal/provider"
)

func (s *Server) distributionTrustedKeys() map[string]ed25519.PublicKey {
	// Production signing keys are not shipped until key custody is decided.
	// An empty map keeps fetch/accept fail-closed.
	return map[string]ed25519.PublicKey{}
}

func (s *Server) handleProviderDistributions(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/api/provider-distributions/")
	switch path {
	case "status":
		s.handleProviderDistributionStatus(w, r)
	case "diff":
		s.handleProviderDistributionDiff(w, r)
	case "check":
		s.handleProviderDistributionCheck(w, r)
	case "accept":
		s.handleProviderDistributionAccept(w, r)
	case "rollback":
		s.handleProviderDistributionRollback(w, r)
	default:
		if !s.guard(w, r, r.Method) {
			return
		}
		writeJSONError(w, http.StatusNotFound, "not_found", "provider distribution endpoint not found")
	}
}

func (s *Server) handleProviderDistributionStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.distributionStore == nil {
		writeJSON(w, map[string]any{"state": "none", "enabled": false, "reason": "distribution_store_unavailable"})
		return
	}
	status, err := s.distributionStore.Status()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "distribution_status_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{
		"state":   status.State,
		"enabled": len(s.distributionTrustedKeys()) > 0,
		"status":  status,
	})
}

func (s *Server) handleProviderDistributionCheck(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if len(s.distributionTrustedKeys()) == 0 {
		writeJSONError(w, http.StatusServiceUnavailable, "distribution_keys_unconfigured", "official catalog signing keys are not configured")
		return
	}
	// Remote fetch itself (choosing and reaching a source URL) is out of
	// scope for this remediation — see the C4 completion note in
	// docs/local/bugfix_provider-registry-adversarial-review-remediation_2026-09-16.md.
	// Once trusted keys exist this still fails closed instead of silently
	// no-op-ing, so enabling keys alone cannot make this endpoint pretend to
	// have fetched something it never did.
	writeJSONError(w, http.StatusServiceUnavailable, "distribution_fetch_disabled", "official catalog fetch is not enabled")
}

// handleProviderDistributionDiff compares the currently accepted
// distribution against a specific downloaded candidate identified by
// ?digest=, so the UI can show what accepting that candidate would change.
// Both loads distinguish "nothing there yet" (a normal state, e.g. no
// distribution has ever been accepted) from a real read failure — a
// corrupted accepted pointer must come back as a diagnostic-bearing error,
// not silently as an empty diff, which is what the previous implementation
// did for every failure (including passing the accepted bundle as both
// "current" and "candidate", so the diff was always empty regardless).
func (s *Server) handleProviderDistributionDiff(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.distributionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "distribution_store_unavailable", "distribution store is unavailable")
		return
	}
	digest := strings.TrimSpace(r.URL.Query().Get("digest"))
	if digest == "" {
		writeJSONError(w, http.StatusBadRequest, "digest_required", "digest is required")
		return
	}
	candidate, err := s.distributionStore.LoadDownloadedBundle(digest)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "distribution_candidate_not_found", err.Error())
		return
	}
	var current []provider.Definition
	accepted, err := s.distributionStore.LoadAcceptedBundle()
	if err != nil {
		if !errors.Is(err, provider.ErrNoAcceptedDistribution) {
			writeJSONError(w, http.StatusInternalServerError, "distribution_accepted_load_failed", err.Error())
			return
		}
	} else {
		current = accepted.Payload.Definitions
	}
	var overrides []provider.Definition
	if s.historyStore != nil {
		loaded, _, loadErr := s.historyStore.LoadOverrides()
		if loadErr != nil {
			writeJSONError(w, http.StatusInternalServerError, "override_load_failed", loadErr.Error())
			return
		}
		overrides = loaded
	}
	writeJSON(w, map[string]any{
		"ok":   true,
		"diff": provider.DiffDistribution(current, candidate.Payload.Definitions, overrides),
	})
}

// handleProviderDistributionAccept promotes a downloaded candidate (by
// digest) to accepted and reloads the Registry so the change takes effect
// immediately for both new session spawns and the provider management UI,
// instead of only becoming visible whenever some unrelated reload next runs.
func (s *Server) handleProviderDistributionAccept(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	trustedKeys := s.distributionTrustedKeys()
	if len(trustedKeys) == 0 {
		writeJSONError(w, http.StatusServiceUnavailable, "distribution_keys_unconfigured", "official catalog signing keys are not configured")
		return
	}
	if s.distributionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "distribution_store_unavailable", "distribution store is unavailable")
		return
	}
	var body struct {
		Digest string `json:"digest"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	digest := strings.TrimSpace(body.Digest)
	if digest == "" {
		writeJSONError(w, http.StatusBadRequest, "digest_required", "digest is required")
		return
	}
	snapshot, err := s.distributionStore.SnapshotPointers()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "distribution_snapshot_failed", err.Error())
		return
	}
	status, err := s.distributionStore.Accept(digest, trustedKeys, s.version)
	if err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "distribution_accept_failed", err.Error())
		return
	}
	diagnostics, reloadErr := s.reloadProviderRegistry()
	if reloadErr != nil {
		if restoreErr := s.distributionStore.RestorePointers(snapshot); restoreErr != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_reload_restore_failed", reloadErr.Error()+"; restore distribution pointers: "+restoreErr.Error())
			return
		}
		_, _ = s.reloadProviderRegistry()
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", reloadErr.Error()+"; distribution acceptance was reverted")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "status": status, "diagnostics": diagnostics})
}

func (s *Server) handleProviderDistributionRollback(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.distributionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "distribution_store_unavailable", "distribution store is unavailable")
		return
	}
	snapshot, err := s.distributionStore.SnapshotPointers()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "distribution_snapshot_failed", err.Error())
		return
	}
	status, err := s.distributionStore.Rollback()
	if err != nil {
		writeJSONError(w, http.StatusUnprocessableEntity, "distribution_rollback_failed", err.Error())
		return
	}
	// Rollback used to leave the cached Registry pointed at the
	// now-superseded accepted distribution until some unrelated reload
	// happened to run — the rolled-back state was correct on disk but not in
	// what the running Hub actually resolved providers from.
	diagnostics, reloadErr := s.reloadProviderRegistry()
	if reloadErr != nil {
		if restoreErr := s.distributionStore.RestorePointers(snapshot); restoreErr != nil {
			writeJSONError(w, http.StatusInternalServerError, "provider_reload_restore_failed", reloadErr.Error()+"; restore distribution pointers: "+restoreErr.Error())
			return
		}
		_, _ = s.reloadProviderRegistry()
		writeJSONError(w, http.StatusInternalServerError, "provider_reload_failed", reloadErr.Error()+"; distribution rollback was reverted")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "status": status, "diagnostics": diagnostics})
}

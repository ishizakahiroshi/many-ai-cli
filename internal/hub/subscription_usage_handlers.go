package hub

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// handleSubscriptionUsage returns only profile names and provider-reported
// usage metadata. Profile directories and authentication material never cross
// this boundary.
func (s *Server) handleSubscriptionUsage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	store := s.refreshSubscriptionUsage()
	response := store.snapshot(s.snapshotCfg())
	s.applyUsageProbeStates(&response)
	writeJSON(w, response)
}

type subscriptionUsageProbeRequest struct {
	Provider string `json:"provider"`
	ID       string `json:"id"`
}

// handleSubscriptionUsageProbe starts or cancels the one-turn Claude TUI
// probe. The POST is intentionally synchronous so the UI gets a definitive
// success/failure result, while DELETE cancels its context from another tab or
// the same dropdown.
func (s *Server) handleSubscriptionUsageProbe(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost, http.MethodDelete) {
		return
	}
	var body subscriptionUsageProbeRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	provider := strings.TrimSpace(body.Provider)
	id := config.NormalizeSubscriptionID(body.ID)
	if provider != "claude" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "usage probe supports Claude profiles only")
		return
	}
	if err := config.ValidateSubscriptionID(id); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	key := usageProbeKey(provider, id)
	if r.Method == http.MethodDelete {
		if s.usageProbe == nil || !s.usageProbe.cancel(key) {
			writeJSONError(w, http.StatusNotFound, "not_found", "usage probe is not running")
			return
		}
		writeJSON(w, map[string]any{"ok": true, "cancelled": true, "provider": provider, "id": id})
		return
	}

	cfg := s.snapshotCfg()
	configDir := subscriptionConfigDir()
	resolved, err := subscription.Resolve(cfg, configDir, provider, id)
	if err != nil {
		writeJSONError(w, subscriptionErrorStatus(err), "invalid_subscription", err.Error())
		return
	}
	if resolved == nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "subscription id is required")
		return
	}
	root, err := ensureUsageProbeRoot(configDir)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "config_dir_error", "cannot prepare usage probe")
		return
	}
	now := time.Now()
	probeCWD := usageProbeCWD(root)
	record := usageProbeRecord{
		Version:   usageProbeMarkerVersion,
		Provider:  provider,
		ProfileID: resolved.ID,
		Label:     "usage-probe-" + resolved.ID + "-" + now.Format("20060102150405.000000000"),
		StartedAt: now,
		CWD:       probeCWD,
	}
	ctx, cancel := context.WithCancel(r.Context())
	state := &usageProbeState{record: record, cancel: cancel}
	if s.usageProbe == nil || !s.usageProbe.begin(key, state) {
		cancel()
		writeJSONError(w, http.StatusConflict, "probe_running", "usage probe is already running for this profile")
		return
	}
	defer func() {
		cancel()
		s.usageProbe.finish(key)
		removeUsageProbeRecord(root, provider, resolved.ID)
	}()
	if err := writeUsageProbeRecord(root, record); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "probe_marker_error", "cannot record usage probe state")
		return
	}

	err = s.runUsageProbe(ctx, key, root, resolved, record)
	if err == nil {
		writeJSON(w, map[string]any{"ok": true, "provider": provider, "id": resolved.ID})
		return
	}
	status := http.StatusBadGateway
	code := "probe_failed"
	if errors.Is(err, context.DeadlineExceeded) {
		status = http.StatusGatewayTimeout
		code = "probe_timeout"
	} else if errors.Is(err, context.Canceled) {
		status = http.StatusRequestTimeout
		code = "probe_cancelled"
	}
	s.logger.Warn("usage probe failed", "provider", provider, "profile_id", resolved.ID, "err", err)
	writeJSONError(w, status, code, "usage could not be retrieved")
}

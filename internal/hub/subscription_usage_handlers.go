package hub

import (
	"context"
	"errors"
	"net/http"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

var codexUsageChecks = struct {
	sync.Mutex
	active map[string]bool
}{active: map[string]bool{}}

func codexUsageDirectoryKey(path string) string {
	clean := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		return strings.ToLower(clean)
	}
	return clean
}

type codexUsageRefreshRequest struct {
	Provider  string `json:"provider"`
	ID        string `json:"id"`
	SessionID int    `json:"session_id,omitempty"`
}

// handleSubscriptionUsage returns only profile names and provider-reported
// usage metadata. Profile directories and authentication material never cross
// this boundary.
func (s *Server) handleSubscriptionUsage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	store := s.refreshSubscriptionUsage()
	cfg := s.snapshotCfg()
	response := store.snapshot(cfg)
	store.applyAuthStatuses(r.Context(), cfg, subscriptionConfigDir(), &response, r.URL.Query().Get("refresh_auth") == "1")
	s.applyUsageProbeStates(&response)
	writeJSON(w, response)
}

// handleCodexUsageRefresh explicitly reads the selected profile's ChatGPT
// limits. It never runs an agent turn or reuses the default Codex login.
func (s *Server) handleCodexUsageRefresh(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body codexUsageRefreshRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	if strings.TrimSpace(body.Provider) != "codex" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "manual refresh supports Codex profiles only")
		return
	}
	id := config.NormalizeSubscriptionID(body.ID)
	if err := config.ValidateSubscriptionID(id); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	resolved, err := subscription.Resolve(s.snapshotCfg(), subscriptionConfigDir(), "codex", id)
	if err != nil || resolved == nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_subscription", "Codex profile is unavailable")
		return
	}
	if body.SessionID != 0 {
		s.sessionsMu.Lock()
		ses := s.sessions[body.SessionID]
		matches := ses != nil && ses.Provider == "codex" &&
			config.NormalizeSubscriptionID(ses.SubscriptionProfileID) == id &&
			codexUsageDirectoryKey(ses.CodexHome) == codexUsageDirectoryKey(resolved.ProfileDir)
		s.sessionsMu.Unlock()
		if !matches {
			writeJSONError(w, http.StatusConflict, "session_profile_changed", "session login directory no longer matches this profile")
			return
		}
	}
	key := codexUsageDirectoryKey(resolved.ProfileDir)
	codexUsageChecks.Lock()
	if codexUsageChecks.active[key] {
		codexUsageChecks.Unlock()
		writeJSONError(w, http.StatusConflict, "refresh_running", "usage refresh is already running for this profile")
		return
	}
	codexUsageChecks.active[key] = true
	codexUsageChecks.Unlock()
	defer func() {
		codexUsageChecks.Lock()
		delete(codexUsageChecks.active, key)
		codexUsageChecks.Unlock()
	}()
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	usage, err := subscription.ReadCodexAppServerUsage(ctx, resolved.ProfileDir)
	if err != nil {
		status := http.StatusBadGateway
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			status = http.StatusGatewayTimeout
		}
		writeJSONError(w, status, "usage_refresh_failed", "Codex usage could not be retrieved; previous values were kept")
		return
	}
	current, err := subscription.Resolve(s.snapshotCfg(), subscriptionConfigDir(), "codex", id)
	if err != nil || current == nil || codexUsageDirectoryKey(current.ProfileDir) != key {
		writeJSONError(w, http.StatusConflict, "profile_changed", "Codex profile changed during usage refresh; previous values were kept")
		return
	}
	checkedAt := time.Now()
	s.subscriptionUsageStoreForServer().putCodexAppServer(id, usage, checkedAt)
	writeJSON(w, map[string]any{"ok": true, "provider": "codex", "id": id, "checked_at": checkedAt.Format(time.RFC3339)})
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

package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/subscription"
)

const (
	usageProbeDirName        = "usage-probe"
	usageProbeActiveDirName  = "active"
	usageProbeMarkerVersion  = 1
	usageProbeTranscriptKeep = 3
	usageProbeSpawnTimeout   = 20 * time.Second
	usageProbeTimeout        = 60 * time.Second
)

// usageProbeRecord is deliberately limited to recovery metadata. It contains
// no credentials, prompt text, account identity, or provider output.
type usageProbeRecord struct {
	Version   int       `json:"version"`
	Provider  string    `json:"provider"`
	ProfileID string    `json:"profile_id"`
	Label     string    `json:"label"`
	SessionID int       `json:"session_id,omitempty"`
	PID       int       `json:"pid,omitempty"`
	StartedAt time.Time `json:"started_at"`
	CWD       string    `json:"cwd"`
}

type usageProbeState struct {
	record usageProbeRecord
	cancel context.CancelFunc
}

type usageProbeManager struct {
	mu     sync.Mutex
	active map[string]*usageProbeState
}

func newUsageProbeManager() *usageProbeManager {
	return &usageProbeManager{active: map[string]*usageProbeState{}}
}

func usageProbeKey(provider, id string) string {
	return strings.ToLower(strings.TrimSpace(provider)) + ":" + config.NormalizeSubscriptionID(id)
}

func (m *usageProbeManager) begin(key string, state *usageProbeState) bool {
	if m == nil || state == nil || strings.TrimSpace(key) == "" {
		return false
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.active == nil {
		m.active = map[string]*usageProbeState{}
	}
	if _, exists := m.active[key]; exists {
		return false
	}
	m.active[key] = state
	return true
}

func (m *usageProbeManager) finish(key string) {
	if m == nil {
		return
	}
	m.mu.Lock()
	delete(m.active, key)
	m.mu.Unlock()
}

func (m *usageProbeManager) cancel(key string) bool {
	if m == nil {
		return false
	}
	m.mu.Lock()
	state := m.active[key]
	m.mu.Unlock()
	if state == nil {
		return false
	}
	if state.cancel != nil {
		state.cancel()
	}
	return true
}

func (m *usageProbeManager) update(key string, fn func(*usageProbeRecord)) (usageProbeRecord, bool) {
	if m == nil || fn == nil {
		return usageProbeRecord{}, false
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	state := m.active[key]
	if state == nil {
		return usageProbeRecord{}, false
	}
	fn(&state.record)
	return state.record, true
}

func (m *usageProbeManager) updateByLabel(label string, fn func(*usageProbeRecord)) (usageProbeRecord, bool) {
	if m == nil || fn == nil || strings.TrimSpace(label) == "" {
		return usageProbeRecord{}, false
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, state := range m.active {
		if state.record.Label != label {
			continue
		}
		fn(&state.record)
		return state.record, true
	}
	return usageProbeRecord{}, false
}

func (m *usageProbeManager) isRunning(provider, id string) bool {
	if m == nil {
		return false
	}
	m.mu.Lock()
	_, ok := m.active[usageProbeKey(provider, id)]
	m.mu.Unlock()
	return ok
}

func usageProbeRoot(configDir string) string {
	return filepath.Join(filepath.Clean(configDir), usageProbeDirName)
}

func ensureUsageProbeRoot(configDir string) (string, error) {
	configDir = strings.TrimSpace(configDir)
	if configDir == "" {
		return "", errors.New("config directory is empty")
	}
	root := usageProbeRoot(configDir)
	if err := os.MkdirAll(filepath.Join(root, usageProbeActiveDirName), sessionlog.PrivateDirMode); err != nil {
		return "", fmt.Errorf("create usage probe directory: %w", err)
	}
	_ = os.Chmod(root, sessionlog.PrivateDirMode)
	_ = os.Chmod(filepath.Join(root, usageProbeActiveDirName), sessionlog.PrivateDirMode)
	return root, nil
}

func usageProbeMarkerPath(root, provider, id string) string {
	return filepath.Join(root, usageProbeActiveDirName, provider+"-"+id+".json")
}

func writeUsageProbeRecord(root string, record usageProbeRecord) error {
	path := usageProbeMarkerPath(root, record.Provider, record.ProfileID)
	dir := filepath.Dir(path)
	data, err := json.MarshalIndent(record, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, "probe-*.tmp")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	_ = os.Chmod(tmpName, sessionlog.PrivateFileMode)
	return os.Rename(tmpName, path)
}

func removeUsageProbeRecord(root, provider, id string) {
	_ = os.Remove(usageProbeMarkerPath(root, provider, id))
}

func usageProbeCWD(root string) string {
	return filepath.Join(root)
}

// cleanupUsageProbeTranscripts operates on one exact Claude projects path. It
// never walks the profile tree and never follows a directory name prefix, so a
// user's real project transcript cannot become a cleanup target.
func cleanupUsageProbeTranscripts(profileDir, probeCWD string) (int, error) {
	profileDir = strings.TrimSpace(profileDir)
	probeCWD = strings.TrimSpace(probeCWD)
	if profileDir == "" || probeCWD == "" {
		return 0, nil
	}
	target := filepath.Join(profileDir, "projects", claudeProjectDirName(probeCWD))
	entries, err := os.ReadDir(target)
	if errors.Is(err, os.ErrNotExist) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	type transcriptFile struct {
		path string
		name string
		mod  time.Time
	}
	files := make([]transcriptFile, 0, len(entries))
	for _, entry := range entries {
		if entry.IsDir() || strings.ToLower(filepath.Ext(entry.Name())) != ".jsonl" {
			continue
		}
		info, statErr := entry.Info()
		if statErr != nil || !info.Mode().IsRegular() {
			continue
		}
		files = append(files, transcriptFile{
			path: filepath.Join(target, entry.Name()),
			name: entry.Name(),
			mod:  info.ModTime(),
		})
	}
	sort.Slice(files, func(i, j int) bool {
		if files[i].mod.Equal(files[j].mod) {
			return files[i].name > files[j].name
		}
		return files[i].mod.After(files[j].mod)
	})
	removed := 0
	keepFrom := usageProbeTranscriptLimit(len(files))
	for _, file := range files[keepFrom:] {
		if err := os.Remove(file.path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return removed, err
		}
		removed++
	}
	return removed, nil
}

func usageProbeTranscriptLimit(count int) int {
	if count <= usageProbeTranscriptKeep {
		return count
	}
	return usageProbeTranscriptKeep
}

func (s *Server) cleanupAllUsageProbeTranscripts(configDir, probeCWD string) {
	cfg := s.snapshotCfg()
	if cfg == nil {
		return
	}
	for _, profile := range cfg.Subscriptions["claude"] {
		if !profile.IsEnabled() {
			continue
		}
		profileDir, err := config.ResolveSubscriptionProfileDir(configDir, "claude", profile)
		if err != nil {
			continue
		}
		if _, err := cleanupUsageProbeTranscripts(profileDir, probeCWD); err != nil {
			s.logger.Warn("usage probe transcript cleanup failed", "provider", "claude", "profile_id", config.NormalizeSubscriptionID(profile.ID), "err", err)
		}
	}
}

func (s *Server) recoverUsageProbes(configDir string) {
	root, err := ensureUsageProbeRoot(configDir)
	if err != nil {
		s.logger.Warn("usage probe recovery unavailable", "err", err)
		return
	}
	activeDir := filepath.Join(root, usageProbeActiveDirName)
	entries, err := os.ReadDir(activeDir)
	if err == nil {
		for _, entry := range entries {
			if entry.IsDir() || strings.ToLower(filepath.Ext(entry.Name())) != ".json" {
				continue
			}
			path := filepath.Join(activeDir, entry.Name())
			data, readErr := os.ReadFile(path)
			var record usageProbeRecord
			valid := readErr == nil && json.Unmarshal(data, &record) == nil &&
				record.Version == usageProbeMarkerVersion &&
				record.Provider == "claude" &&
				config.ValidateSubscriptionID(config.NormalizeSubscriptionID(record.ProfileID)) == nil &&
				record.CWD == usageProbeCWD(root) &&
				strings.HasPrefix(record.Label, "usage-probe-")
			if valid && record.PID > 0 && record.PID != os.Getpid() {
				if process, findErr := os.FindProcess(record.PID); findErr == nil {
					_ = process.Kill()
				}
				s.logger.Info("usage probe process recovered", "provider", record.Provider, "profile_id", record.ProfileID, "pid", record.PID)
			}
			_ = os.Remove(path)
		}
	}
	s.cleanupAllUsageProbeTranscripts(configDir, usageProbeCWD(root))
}

func (s *Server) persistActiveUsageProbe(key string, root string, fn func(*usageProbeRecord)) {
	if s.usageProbe == nil || fn == nil {
		return
	}
	record, ok := s.usageProbe.update(key, fn)
	if !ok {
		return
	}
	if err := writeUsageProbeRecord(root, record); err != nil {
		s.logger.Warn("usage probe marker update failed", "provider", record.Provider, "profile_id", record.ProfileID, "err", err)
	}
}

func (s *Server) updateUsageProbePID(label string, pid int) {
	if s.usageProbe == nil || pid <= 0 {
		return
	}
	record, ok := s.usageProbe.updateByLabel(label, func(record *usageProbeRecord) {
		record.PID = pid
	})
	if !ok {
		return
	}
	if dir := subscriptionConfigDir(); dir != "" {
		if root, err := ensureUsageProbeRoot(dir); err == nil {
			if err := writeUsageProbeRecord(root, record); err != nil {
				s.logger.Warn("usage probe pid marker update failed", "provider", record.Provider, "profile_id", record.ProfileID, "err", err)
			}
		}
	}
}

func (s *Server) usageProbeModel() string {
	cfg := s.snapshotCfg()
	if cfg == nil {
		return config.DefaultUsageProbeModel
	}
	return config.EffectiveUsageProbeModel(cfg.UserPrefs.UsageProbeModel)
}

func (s *Server) sessionPID(sessionID int) int {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if wrapper := s.wrappers[sessionID]; wrapper != nil {
		return wrapper.pid
	}
	return 0
}

func (s *Server) probeSessionActive(sessionID int) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	_, ok := s.sessions[sessionID]
	return ok
}

func (s *Server) dismissUsageProbeSession(sessionID int) {
	if sessionID <= 0 || !s.probeSessionActive(sessionID) {
		return
	}
	_ = s.handleDismiss(protoMessageSessionDismiss(sessionID))
}

// protoMessageSessionDismiss keeps the probe lifecycle independent from the
// UI request path while using the same wrapper close and cleanup semantics.
func protoMessageSessionDismiss(sessionID int) proto.Message {
	return proto.Message{Type: "session_dismiss", SessionID: sessionID}
}

func hasClaudeRateLimit(stat *usageStat) bool {
	return stat != nil && (stat.RateLimit5hReset > 0 || stat.RateLimit7dReset > 0)
}

func (s *Server) waitForUsageProbeResult(ctx context.Context, sessionID int) error {
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	for {
		if hasClaudeRateLimit(GetSessionUsageStat(sessionID)) {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}

func (s *Server) runUsageProbe(ctx context.Context, key, root string, resolved *subscription.Resolved, record usageProbeRecord) error {
	if resolved == nil || resolved.Provider != "claude" {
		return errors.New("usage probe supports Claude profiles only")
	}
	if ctx == nil {
		ctx = context.Background()
	}
	if _, err := cleanupUsageProbeTranscripts(resolved.ProfileDir, record.CWD); err != nil {
		return fmt.Errorf("cleanup before probe: %w", err)
	}
	probeCtx, cancel := context.WithTimeout(ctx, usageProbeTimeout)
	defer cancel()
	sessionID, err := s.spawnWrappedSession(spawnWrappedSpec{
		Context:               probeCtx,
		Provider:              "claude",
		CWD:                   record.CWD,
		Model:                 s.usageProbeModel(),
		Label:                 record.Label,
		SubscriptionProfileID: resolved.ID,
		UsageProbe:            true,
	}, usageProbeSpawnTimeout)
	if err != nil {
		// 実エラーを捨てると hub ログでも原因が分からなくなる（起動前の検証で
		// 落ちたのか、wrapper が登録されなかったのかが区別できない）。
		// クライアントへ返す文言はハンドラ側で固定しているので、ここは wrap する。
		return fmt.Errorf("could not start usage probe: %w", err)
	}
	s.persistActiveUsageProbe(key, root, func(current *usageProbeRecord) {
		current.SessionID = sessionID
		if pid := s.sessionPID(sessionID); pid > 0 {
			current.PID = pid
		}
	})
	defer func() {
		s.dismissUsageProbeSession(sessionID)
		if _, cleanupErr := cleanupUsageProbeTranscripts(resolved.ProfileDir, record.CWD); cleanupErr != nil {
			s.logger.Warn("usage probe transcript cleanup failed", "provider", "claude", "profile_id", resolved.ID, "err", cleanupErr)
		}
	}()
	// The wrapper registers before the provider TUI is ready. Give the input
	// path the same bounded quiet-period grace used by orchestration prompts.
	s.waitForInputReady(sessionID, 300*time.Millisecond, 10*time.Second)
	if err := probeCtx.Err(); err != nil {
		return err
	}
	s.injectText(sessionID, "ok", true, false)
	return s.waitForUsageProbeResult(probeCtx, sessionID)
}

// applyUsageProbeStates adds only the transient state needed by the UI. It is
// intentionally separate from the usage store so a failed/cancelled probe
// cannot erase the last successful usage value.
func (s *Server) applyUsageProbeStates(response *subscriptionUsageResponse) {
	if response == nil || s.usageProbe == nil {
		return
	}
	for providerIndex := range response.Providers {
		provider := &response.Providers[providerIndex]
		for profileIndex := range provider.Profiles {
			profile := &provider.Profiles[profileIndex]
			if s.usageProbe.isRunning(provider.Provider, profile.ID) {
				profile.ProbeState = "running"
			}
		}
	}
}

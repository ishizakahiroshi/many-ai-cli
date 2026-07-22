package hub

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	notifyPkg "many-ai-cli/internal/notify"
)

// validateNotifyBackend は通知バックエンド設定の妥当性を検証する（finding #35）。
// type: "ntfy" または "webhook" のみ許可。
// url: http/https スキーム・ホスト必須。空・ファイル/ftp スキーム等は拒否。
// ntfy: topic が空の場合も拒否。
func validateNotifyBackend(backend config.NotifyBackendConfig) error {
	backendType := strings.TrimSpace(backend.Type)
	if backendType != "ntfy" && backendType != "webhook" {
		return fmt.Errorf("invalid backend type %q: must be ntfy or webhook", backend.Type)
	}
	rawURL := strings.TrimSpace(backend.URL)
	if rawURL == "" {
		return fmt.Errorf("backend URL is required")
	}
	parsed, err := url.Parse(rawURL)
	if err != nil {
		return fmt.Errorf("backend URL is invalid: %w", err)
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return fmt.Errorf("backend URL scheme %q is not allowed: must be http or https", parsed.Scheme)
	}
	if parsed.Host == "" {
		return fmt.Errorf("backend URL must have a host")
	}
	if backendType == "ntfy" && strings.TrimSpace(backend.Topic) == "" {
		return fmt.Errorf("ntfy backend requires a non-empty topic")
	}
	return nil
}

func (s *Server) handleLogConfig(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		logCfg := s.cfg.Log
		logDir := s.cfg.Hub.LogDir
		s.cfgMu.Unlock()
		cfgDir, _ := config.Dir()
		attachDir := filepath.Join(cfgDir, "attachments")
		type logConfigResp struct {
			config.LogConfig
			LogDir    string `json:"log_dir"`
			AttachDir string `json:"attach_dir"`
		}
		writeJSON(w, logConfigResp{logCfg, logDir, attachDir})
	case http.MethodPost:
		var body config.LogConfig
		if !decodeJSON(w, r, &body) {
			return
		}
		if body.MaxSizeMB < 1 {
			body.MaxSizeMB = 1
		} else if body.MaxSizeMB > 1000 {
			body.MaxSizeMB = 1000
		}
		if body.MaxBackups < 0 {
			body.MaxBackups = 0
		} else if body.MaxBackups > 100 {
			body.MaxBackups = 100
		}
		if body.SessionRetentionDays < 0 {
			body.SessionRetentionDays = 0
		} else if body.SessionRetentionDays > 365 {
			body.SessionRetentionDays = 365
		}
		if body.SessionMaxSizeMB < 0 {
			body.SessionMaxSizeMB = 0
		} else if body.SessionMaxSizeMB > 10000 {
			body.SessionMaxSizeMB = 10000
		}
		if body.AttachmentRetentionDays < 0 {
			body.AttachmentRetentionDays = 0
		} else if body.AttachmentRetentionDays > 365 {
			body.AttachmentRetentionDays = 365
		}
		if body.AttachmentMaxTotalMB < 0 {
			body.AttachmentMaxTotalMB = 0
		} else if body.AttachmentMaxTotalMB > 100000 {
			body.AttachmentMaxTotalMB = 100000
		}
		s.cfgMu.Lock()
		// LegacyLogsNoticeShown はサーバ管理フラグで設定フォームには含まれないため、
		// body の零値で上書きせず現在値を引き継ぐ（さもないと旧ログ通知が再表示され得る）。
		body.LegacyLogsNoticeShown = s.cfg.Log.LegacyLogsNoticeShown
		s.cfg.Log = body
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}
func (s *Server) handleIdleTimeout(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		min := s.cfg.Hub.IdleTimeoutMin
		s.cfgMu.Unlock()
		writeJSON(w, map[string]int{"idle_timeout_min": min})
	case http.MethodPost:
		var body struct {
			IdleTimeoutMin int `json:"idle_timeout_min"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if body.IdleTimeoutMin < 0 {
			body.IdleTimeoutMin = 0
		} else if body.IdleTimeoutMin > 1440 {
			body.IdleTimeoutMin = 1440
		}
		s.cfgMu.Lock()
		s.cfg.Hub.IdleTimeoutMin = body.IdleTimeoutMin
		s.cfgMu.Unlock()
		// タイマーは一旦止めて新しい閾値で再構成する。UI 接続中はカウントダウン
		// しないため再開しない（接続が無いときだけ再始動する）。
		s.sessionsMu.Lock()
		s.stopIdleTimerLocked()
		if len(s.uis) == 0 {
			s.startIdleTimerLocked(body.IdleTimeoutMin)
		}
		s.sessionsMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleNotifyConfig は GET/POST で ntfy/webhook 通知設定を読み書きする。
// POST body: { backends: [...], events: [...] }
func (s *Server) handleNotifyConfig(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		nc := s.cfg.Notify
		s.cfgMu.Unlock()
		writeJSON(w, nc)
	case http.MethodPost:
		var body config.NotifyConfig
		if !decodeJSON(w, r, &body) {
			return
		}
		// バリデーション: /api/notify-test 用の validateNotifyBackend を永続化側でも
		// 通し、handleNotifyTest と非対称にならないようにする。無効な backend が
		// config.yaml に残ると次回起動から silent send-fail になり切り分けが困難。
		for _, b := range body.Backends {
			if err := validateNotifyBackend(b); err != nil {
				writeJSONError(w, http.StatusBadRequest, "invalid_backend", errorDetail("backend validation failed", err))
				return
			}
		}
		s.cfgMu.Lock()
		s.cfg.Notify = body
		s.cfgMu.Unlock()
		// notifyMgr の設定を動的反映
		if s.notifyMgr != nil {
			s.notifyMgr.UpdateConfig(configToNotify(body))
		}
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleNotifyTest は Settings の「テスト送信」ボタン用。
// POST body: { backend: { type, url, topic } }
func (s *Server) handleNotifyTest(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Backend config.NotifyBackendConfig `json:"backend"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	// finding #35 の validateNotifyBackend を配線する（従来はここで Type のみ検査し、
	// 用意された validator が呼ばれず dead code だった）。テスト送信は完全に設定済みの
	// バックエンドに対してのみ意味を持つため、scheme/host/topic を満たさない要求を
	// 送信前に 400 で弾く（未設定バックエンドのテストは元々成功しないため非破壊）。
	// 注: private ネット宛の SSRF 遮断はここでは行わない。ntfy/webhook は localhost/LAN
	// への自己ホストが正規ユースケースであり、一律ブロックは機能を壊すため（進言参照）。
	if err := validateNotifyBackend(body.Backend); err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_backend", err.Error())
		return
	}
	bc := notifyPkg.BackendConfig{
		Type:  body.Backend.Type,
		URL:   body.Backend.URL,
		Topic: body.Backend.Topic,
	}
	ctx, cancel := context.WithTimeout(r.Context(), 10*time.Second)
	defer cancel()
	if s.notifyMgr == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "notify_unavailable", "notify manager not initialized")
		return
	}
	if err := s.notifyMgr.SendTest(ctx, bc, "many-ai-cli test", "Test notification from many-ai-cli Hub"); err != nil {
		writeJSONError(w, http.StatusBadGateway, "send_failed", err.Error())
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

// handleNotifyGenerateTopic は ntfy トピックのランダム生成 API。
func (s *Server) handleNotifyGenerateTopic(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	topic, err := notifyPkg.GenerateRandomTopic()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "generate_failed", err.Error())
		return
	}
	writeJSON(w, map[string]string{"topic": topic})
}

func (s *Server) handleReconnectGrace(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		sec := s.cfg.Hub.WrapperReconnectGraceSec
		s.cfgMu.Unlock()
		writeJSON(w, map[string]int{"wrapper_reconnect_grace_sec": sec})
	case http.MethodPost:
		var body struct {
			WrapperReconnectGraceSec int `json:"wrapper_reconnect_grace_sec"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if body.WrapperReconnectGraceSec < 0 {
			body.WrapperReconnectGraceSec = 0
		} else if body.WrapperReconnectGraceSec > 86400 {
			body.WrapperReconnectGraceSec = 86400
		}
		s.cfgMu.Lock()
		s.cfg.Hub.WrapperReconnectGraceSec = body.WrapperReconnectGraceSec
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleInputConfig exposes the bounded, UI-safe input timing settings.
// 0 deliberately means "provider default" so new providers keep their own safe baseline.
func (s *Server) handleInputConfig(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		ms := s.cfg.Input.DeferredEnterMS
		s.cfgMu.Unlock()
		writeJSON(w, map[string]int{"deferred_enter_ms": ms})
	case http.MethodPost:
		var body struct {
			DeferredEnterMS int `json:"deferred_enter_ms"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if body.DeferredEnterMS < 0 || body.DeferredEnterMS > 10000 {
			writeJSONError(w, http.StatusBadRequest, "invalid_deferred_enter_ms", "deferred_enter_ms must be between 0 and 10000")
			return
		}
		s.cfgMu.Lock()
		s.cfg.Input.DeferredEnterMS = body.DeferredEnterMS
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleOrchestrationConfig exposes only the UI-safe orchestration preference.
// Limits and worktree fields intentionally remain config-file managed.
func (s *Server) handleOrchestrationConfig(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		mode := config.EffectiveBoardNotifyMode(s.cfg.Orchestration.BoardNotifyMode)
		spawnMode := config.EffectiveSpawnConfirmMode(s.cfg.Orchestration.SpawnConfirmMode)
		providers := append([]string(nil), s.cfg.Orchestration.SpawnConfirmProviders...)
		childTimeout := s.cfg.Orchestration.ChildTimeoutSeconds
		timeoutRespawn := s.cfg.Orchestration.TimeoutRespawn
		s.cfgMu.Unlock()
		writeJSON(w, map[string]any{"board_notify_mode": string(mode), "spawn_confirm_mode": string(spawnMode), "spawn_confirm_providers": providers, "child_timeout_seconds": childTimeout, "timeout_respawn": timeoutRespawn})
	case http.MethodPost:
		var body struct {
			BoardNotifyMode       config.BoardNotifyMode  `json:"board_notify_mode"`
			SpawnConfirmMode      config.SpawnConfirmMode `json:"spawn_confirm_mode"`
			SpawnConfirmProviders []string                `json:"spawn_confirm_providers"`
			ChildTimeoutSeconds   int                     `json:"child_timeout_seconds"`
			TimeoutRespawn        bool                    `json:"timeout_respawn"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if !body.BoardNotifyMode.Valid() || !body.SpawnConfirmMode.Valid() {
			writeJSONError(w, http.StatusBadRequest, "invalid_board_notify_mode", "board_notify_mode must be soft-notify, queue-until-idle, or interrupt")
			return
		}
		if body.ChildTimeoutSeconds < 60 || body.ChildTimeoutSeconds > 86400 {
			writeJSONError(w, http.StatusBadRequest, "invalid_child_timeout", "child_timeout_seconds must be between 60 and 86400")
			return
		}
		s.cfgMu.Lock()
		s.cfg.Orchestration.BoardNotifyMode = body.BoardNotifyMode
		s.cfg.Orchestration.SpawnConfirmMode = body.SpawnConfirmMode
		s.cfg.Orchestration.SpawnConfirmProviders = append([]string(nil), body.SpawnConfirmProviders...)
		s.cfg.Orchestration.ChildTimeoutSeconds = body.ChildTimeoutSeconds
		s.cfg.Orchestration.TimeoutRespawn = body.TimeoutRespawn
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

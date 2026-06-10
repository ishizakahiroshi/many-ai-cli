package hub

import (
	"context"
	"net/http"
	"path/filepath"
	"time"

	"any-ai-cli/internal/config"
	notifyPkg "any-ai-cli/internal/notify"
)

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
		// 簡易バリデーション
		for _, b := range body.Backends {
			if b.Type != "ntfy" && b.Type != "webhook" {
				writeJSONError(w, http.StatusBadRequest, "invalid_type", "backend type must be ntfy or webhook")
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
	if body.Backend.Type != "ntfy" && body.Backend.Type != "webhook" {
		writeJSONError(w, http.StatusBadRequest, "invalid_type", "backend type must be ntfy or webhook")
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
	if err := s.notifyMgr.SendTest(ctx, bc, "any-ai-cli test", "Test notification from any-ai-cli Hub"); err != nil {
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

package hub

import (
	"net/http"
	"path/filepath"

	"any-ai-cli/internal/config"
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
		s.cfgMu.Lock()
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

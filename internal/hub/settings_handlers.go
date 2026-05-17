package hub

import (
	"net/http"
	"os"
	"path/filepath"

	"any-ai-cli/internal/config"
)

func (s *Server) handleLogConfig(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		logCfg := s.cfg.Log
		logDir := s.cfg.Hub.LogDir
		s.mu.Unlock()
		home, _ := os.UserHomeDir()
		attachDir := filepath.Join(home, ".any-ai-cli", "attachments")
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
		s.mu.Lock()
		s.cfg.Log = body
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}
func (s *Server) handleIdleTimeout(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		min := s.cfg.Hub.IdleTimeoutMin
		s.mu.Unlock()
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
		s.mu.Lock()
		s.cfg.Hub.IdleTimeoutMin = body.IdleTimeoutMin
		if len(s.uis) > 0 {
			s.stopIdleTimerLocked()
		} else {
			s.stopIdleTimerLocked()
			s.startIdleTimerLocked()
		}
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}
func (s *Server) handleReconnectGrace(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		sec := s.cfg.Hub.WrapperReconnectGraceSec
		s.mu.Unlock()
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
		s.mu.Lock()
		s.cfg.Hub.WrapperReconnectGraceSec = body.WrapperReconnectGraceSec
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

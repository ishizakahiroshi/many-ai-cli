package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"

	"ai-cli-hub/internal/config"
	"ai-cli-hub/internal/wrapper"
)

// globalRulePaths はグローバルな Claude/Codex ルールファイルのパスを返す
func globalRulePaths() map[string]string {
	home, _ := os.UserHomeDir()
	codexHome := os.Getenv("CODEX_HOME")
	if codexHome == "" {
		codexHome = filepath.Join(home, ".codex")
	}
	return map[string]string{
		"claude": filepath.Join(home, ".claude", "CLAUDE.md"),
		"codex":  filepath.Join(codexHome, "AGENTS.md"),
	}
}

func (s *Server) injectApprovalRules() {
	if err := wrapper.SyncRulesFile(); err != nil {
		s.logger.Warn("sync rules file failed", "err", err)
		return
	}
	for provider, path := range globalRulePaths() {
		if _, err := os.Stat(path); os.IsNotExist(err) {
			continue
		}
		var already bool
		switch provider {
		case "claude":
			already, _ = wrapper.ScanClaudeConfigured(path)
		case "codex":
			already, _ = wrapper.ScanCodexConfigured(path)
		}
		if already {
			continue
		}
		if err := wrapper.InjectRules(provider, path); err != nil {
			s.logger.Warn("inject rules failed", "provider", provider, "path", path, "err", err)
		} else {
			s.logger.Info("inject rules ok", "provider", provider, "path", path)
		}
	}
}

func (s *Server) removeApprovalRules() {
	for provider, path := range globalRulePaths() {
		if err := wrapper.RemoveRules(provider, path); err != nil {
			s.logger.Warn("remove rules failed", "provider", provider, "path", path, "err", err)
		}
	}
}

func (s *Server) handleApprovalStatus(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	s.mu.Lock()
	enabled := s.cfg.Approval.Enabled
	firstLaunchShown := s.cfg.Approval.FirstLaunchShown
	s.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{
		"enabled":            enabled,
		"first_launch_shown": firstLaunchShown,
	})
}

func (s *Server) handleApprovalEnable(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	s.mu.Lock()
	s.cfg.Approval.Enabled = true
	s.cfg.Approval.FirstLaunchShown = true
	s.mu.Unlock()
	s.injectApprovalRules()
	if err := config.Save(s.cfg); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) handleApprovalDisable(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	s.mu.Lock()
	s.cfg.Approval.Enabled = false
	s.mu.Unlock()
	s.removeApprovalRules()
	if err := config.Save(s.cfg); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

// handleApprovalDismiss は初回トーストを「後で」で閉じた際に first_launch_shown をマークする
func (s *Server) handleApprovalDismiss(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	s.mu.Lock()
	s.cfg.Approval.FirstLaunchShown = true
	s.mu.Unlock()
	if err := config.Save(s.cfg); err != nil {
		s.logger.Warn("save config failed", "err", err)
	}
	w.WriteHeader(http.StatusNoContent)
}

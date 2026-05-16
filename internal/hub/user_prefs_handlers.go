package hub

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"

	"any-ai-cli/internal/config"
)

// notifySoundCustomPath はカスタム通知音のバイナリファイルパスを返す。
func notifySoundCustomPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".any-ai-cli", "notify_sound_custom.bin"), nil
}

// handleUserPrefs は GET / PUT /api/user-prefs を処理する。
func (s *Server) handleUserPrefs(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleUserPrefsGet(w, r)
	case http.MethodPut:
		s.handleUserPrefsPut(w, r)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleUserPrefsGet(w http.ResponseWriter, _ *http.Request) {
	s.mu.Lock()
	prefs := s.cfg.UserPrefs
	s.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(prefs)
}

func (s *Server) handleUserPrefsPut(w http.ResponseWriter, r *http.Request) {
	var prefs config.UserPrefs
	if err := json.NewDecoder(r.Body).Decode(&prefs); err != nil {
		http.Error(w, "bad request: "+err.Error(), http.StatusBadRequest)
		return
	}
	s.mu.Lock()
	s.cfg.UserPrefs = prefs
	s.mu.Unlock()
	if err := config.Save(s.cfg); err != nil {
		s.logger.Warn("save config failed", "err", err)
		http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
		return
	}
	s.mu.Lock()
	saved := s.cfg.UserPrefs
	s.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(saved)
}

// handleUserPrefsNotifySoundCustom は GET / PUT /api/user-prefs/notify-sound-custom を処理する。
func (s *Server) handleUserPrefsNotifySoundCustom(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleUserPrefsNotifySoundCustomGet(w, r)
	case http.MethodPut:
		s.handleUserPrefsNotifySoundCustomPut(w, r)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleUserPrefsNotifySoundCustomGet(w http.ResponseWriter, _ *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		http.Error(w, "home dir error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}
	if err != nil {
		http.Error(w, "read error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	s.mu.Lock()
	mime := s.cfg.UserPrefs.NotifySound.CustomMime
	s.mu.Unlock()
	if mime == "" {
		mime = "application/octet-stream"
	}
	w.Header().Set("Content-Type", mime)
	_, _ = w.Write(data)
}

func (s *Server) handleUserPrefsNotifySoundCustomPut(w http.ResponseWriter, r *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		http.Error(w, "home dir error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	data, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, "read body error: "+err.Error(), http.StatusBadRequest)
		return
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		http.Error(w, "write error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	mime := r.Header.Get("Content-Type")
	if mime == "" {
		mime = "application/octet-stream"
	}
	s.mu.Lock()
	s.cfg.UserPrefs.NotifySound.CustomFile = path
	s.cfg.UserPrefs.NotifySound.CustomMime = mime
	s.mu.Unlock()
	if err := config.Save(s.cfg); err != nil {
		s.logger.Warn("save config failed", "err", err)
		http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

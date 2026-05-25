package hub

import (
	"net/http"
	"path/filepath"
	"strings"
)

// checkOpenPathAllowed は open 系ハンドラで path が allowed-roots 配下かを検証する。
// 許可外のパスは 403 を返す。
func (s *Server) checkOpenPathAllowed(w http.ResponseWriter, path string) bool {
	cwd := s.hubCWD
	gitRoot := findGitRoot(cwd)
	allowed, err := isPathUnderAllowedRoots(path, cwd, gitRoot)
	if err != nil || !allowed {
		http.Error(w, "forbidden: path is outside allowed roots", http.StatusForbidden)
		return false
	}
	return true
}

func (s *Server) handleOpenFile(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) || !requireMethod(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Path == "" {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if !s.checkOpenPathAllowed(w, body.Path) {
		return
	}
	s.mu.Lock()
	app := s.cfg.FileOpenApp
	s.mu.Unlock()
	if err := openFileNative(body.Path, app); err != nil {
		writeJSON(w, map[string]any{"ok": false, "error": err.Error()})
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenDefaultFile(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) || !requireMethod(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Path == "" {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if !s.checkOpenPathAllowed(w, body.Path) {
		return
	}
	if err := openFileNative(body.Path, ""); err != nil {
		writeJSON(w, map[string]any{"ok": false, "error": err.Error()})
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenFolder(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) || !requireMethod(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Path == "" {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if !s.checkOpenPathAllowed(w, body.Path) {
		return
	}
	dir := filepath.Dir(body.Path)
	if err := openDirNative(dir); err != nil {
		writeJSON(w, map[string]any{"ok": false, "error": err.Error()})
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenTerminal(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) || !requireMethod(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Path == "" {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if !s.checkOpenPathAllowed(w, body.Path) {
		return
	}
	s.mu.Lock()
	app := s.cfg.TerminalApp
	s.mu.Unlock()
	if err := openTerminalNative(body.Path, app); err != nil {
		writeJSON(w, map[string]any{"ok": false, "error": err.Error()})
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleFileOpenApp(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		app := s.cfg.FileOpenApp
		s.mu.Unlock()
		writeJSON(w, map[string]string{
			"file_open_app":           app,
			"effective_file_open_app": effectiveFileOpenAppDescription(app),
		})
	case http.MethodPost:
		var body struct {
			FileOpenApp string `json:"file_open_app"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		s.mu.Lock()
		s.cfg.FileOpenApp = strings.TrimSpace(body.FileOpenApp)
		s.mu.Unlock()
		if err := s.persistConfig(); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		writeJSON(w, map[string]any{
			"ok":                      true,
			"file_open_app":           s.cfg.FileOpenApp,
			"effective_file_open_app": effectiveFileOpenAppDescription(s.cfg.FileOpenApp),
		})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleTerminalApp(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		app := s.cfg.TerminalApp
		s.mu.Unlock()
		writeJSON(w, map[string]string{
			"terminal_app":           app,
			"effective_terminal_app": effectiveTerminalAppDescription(app),
		})
	case http.MethodPost:
		var body struct {
			TerminalApp string `json:"terminal_app"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		s.mu.Lock()
		s.cfg.TerminalApp = strings.TrimSpace(body.TerminalApp)
		s.mu.Unlock()
		if err := s.persistConfig(); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		writeJSON(w, map[string]any{
			"ok":                     true,
			"terminal_app":           s.cfg.TerminalApp,
			"effective_terminal_app": effectiveTerminalAppDescription(s.cfg.TerminalApp),
		})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

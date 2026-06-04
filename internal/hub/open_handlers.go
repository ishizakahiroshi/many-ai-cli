package hub

import (
	"net/http"
	"path/filepath"
	"strings"
)

// checkOpenPathAllowed は open 系ハンドラで path が allowed-roots 配下かを検証する。
// ?session=<id> が指定されていればそのセッションの CWD を判定基準にする（省略時: Hub cwd）。
// 許可外のパスは 403 を返す。
func (s *Server) checkOpenPathAllowed(w http.ResponseWriter, r *http.Request, path string) bool {
	if !filepath.IsAbs(path) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path must be absolute")
		return false
	}
	cwd := s.cwdForRequest(r)
	gitRoot := findGitRoot(cwd)
	allowed, err := isPathUnderAllowedRoots(path, cwd, gitRoot)
	if err != nil || !allowed {
		writeJSONError(w, http.StatusForbidden, "forbidden", "path is outside allowed roots")
		return false
	}
	return true
}

func (s *Server) decodeAllowedPath(w http.ResponseWriter, r *http.Request) (string, bool) {
	var body struct {
		Path string `json:"path"`
	}
	if !decodeJSON(w, r, &body) {
		return "", false
	}
	if body.Path == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "path is required")
		return "", false
	}
	if !s.checkOpenPathAllowed(w, r, body.Path) {
		return "", false
	}
	return body.Path, true
}

func (s *Server) handleOpenFile(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	s.cfgMu.Lock()
	app := s.cfg.FileOpenApp
	s.cfgMu.Unlock()
	if err := openFileNative(path, app); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenDefaultFile(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	if err := openFileNative(path, ""); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenFolder(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	dir := filepath.Dir(path)
	if err := openDirNative(dir); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleOpenTerminal(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	path, ok := s.decodeAllowedPath(w, r)
	if !ok {
		return
	}
	s.cfgMu.Lock()
	app := s.cfg.TerminalApp
	s.cfgMu.Unlock()
	if err := openTerminalNative(path, app); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", err.Error())
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) handleFileOpenApp(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		app := s.cfg.FileOpenApp
		s.cfgMu.Unlock()
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
		app := strings.TrimSpace(body.FileOpenApp)
		s.cfgMu.Lock()
		s.cfg.FileOpenApp = app
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]any{
			"ok":                      true,
			"file_open_app":           app,
			"effective_file_open_app": effectiveFileOpenAppDescription(app),
		})
	}
}

func (s *Server) handleTerminalApp(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		app := s.cfg.TerminalApp
		s.cfgMu.Unlock()
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
		app := strings.TrimSpace(body.TerminalApp)
		s.cfgMu.Lock()
		s.cfg.TerminalApp = app
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]any{
			"ok":                     true,
			"terminal_app":           app,
			"effective_terminal_app": effectiveTerminalAppDescription(app),
		})
	}
}

package hub

import (
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/config"
)

// notifySoundCustomPath はカスタム通知音のバイナリファイルパスを返す。
func notifySoundCustomPath() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "notify_sound_custom.bin"), nil
}

// avatarUploadPath はアップロードされたアバター画像の保存パスを返す。
func avatarUploadPath() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "user_avatar.bin"), nil
}

// handleUserPrefsAvatarUpload は PUT / DELETE /api/user-prefs/avatar を処理する。
// PUT: バイナリ画像を保存し UserPrefs.Avatar をローカルパスに設定する。
// DELETE: UserPrefs.Avatar を空にする。
func (s *Server) handleUserPrefsAvatarUpload(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPut, http.MethodDelete) {
		return
	}
	switch r.Method {
	case http.MethodPut:
		path, err := avatarUploadPath()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
			return
		}
		r.Body = http.MaxBytesReader(w, r.Body, avatarMaxBytes)
		data, err := io.ReadAll(r.Body)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("read body error", err))
			return
		}
		if err := os.WriteFile(path, data, 0o600); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "write_error", errorDetail("write error", err))
			return
		}
		s.cfgMu.Lock()
		s.cfg.UserPrefs.Avatar = path
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	case http.MethodDelete:
		s.cfgMu.Lock()
		s.cfg.UserPrefs.Avatar = ""
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleUserPrefs は GET / PUT /api/user-prefs を処理する。
func (s *Server) handleUserPrefs(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleUserPrefsGet(w, r)
	case http.MethodPut:
		s.handleUserPrefsPut(w, r)
	}
}

func (s *Server) handleUserPrefsGet(w http.ResponseWriter, _ *http.Request) {
	s.cfgMu.Lock()
	prefs := s.cfg.UserPrefs.Clone()
	s.cfgMu.Unlock()
	writeJSON(w, prefs)
}

// sanitizeAvatarPref は永続化前に Avatar フィールドを正規化する。
// 許可するのは ""（未設定）/ http(s):// URL / アップロード専用パス（user_avatar.bin）
// のみ。任意ローカル絶対パスは "" に落とす（config.yaml 等を avatar 値に仕込んで
// /api/avatar 経由で read する経路を、書き込み側でも塞ぐ defense-in-depth）。
func sanitizeAvatarPref(avatar string) string {
	avatar = strings.TrimSpace(avatar)
	if avatar == "" {
		return ""
	}
	if strings.HasPrefix(avatar, "http://") || strings.HasPrefix(avatar, "https://") {
		return avatar
	}
	if uploadPath, err := avatarUploadPath(); err == nil && pathExistsCandidateKey(avatar) == pathExistsCandidateKey(uploadPath) {
		return avatar
	}
	return ""
}

func (s *Server) handleUserPrefsPut(w http.ResponseWriter, r *http.Request) {
	var prefs config.UserPrefs
	if !decodeJSON(w, r, &prefs) {
		return
	}
	prefs.Avatar = sanitizeAvatarPref(prefs.Avatar)
	s.cfgMu.Lock()
	s.cfg.UserPrefs = prefs
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}
	s.cfgMu.Lock()
	saved := s.cfg.UserPrefs.Clone()
	s.cfgMu.Unlock()
	writeJSON(w, saved)
}

// handleUserPrefsNotifySoundCustom は GET / PUT /api/user-prefs/notify-sound-custom を処理する。
func (s *Server) handleUserPrefsNotifySoundCustom(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleUserPrefsNotifySoundCustomGet(w, r)
	case http.MethodPut:
		s.handleUserPrefsNotifySoundCustomPut(w, r)
	}
}

func (s *Server) handleUserPrefsNotifySoundCustomGet(w http.ResponseWriter, _ *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
		return
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
		return
	}
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_error", errorDetail("read error", err))
		return
	}
	s.cfgMu.Lock()
	mime := s.cfg.UserPrefs.NotifySound.CustomMime
	s.cfgMu.Unlock()
	if mime == "" {
		mime = "application/octet-stream"
	}
	w.Header().Set("Content-Type", mime)
	_, _ = w.Write(data)
}

func (s *Server) handleUserPrefsNotifySoundCustomPut(w http.ResponseWriter, r *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, notifySoundMaxBytes)
	data, err := io.ReadAll(r.Body)
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("read body error", err))
		return
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "write_error", errorDetail("write error", err))
		return
	}
	mime := r.Header.Get("Content-Type")
	if mime == "" {
		mime = "application/octet-stream"
	}
	s.cfgMu.Lock()
	s.cfg.UserPrefs.NotifySound.CustomFile = path
	s.cfg.UserPrefs.NotifySound.CustomMime = mime
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

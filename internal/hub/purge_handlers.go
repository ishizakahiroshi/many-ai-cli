package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// activeLogBases は稼働中セッションのログ三つ組（.log/.jsonl/.txt）の
// 共通ベースパス（拡張子なし）集合を返す。全消し時にこのベースを持つ
// ファイルを保護対象として除外するために使う。
func (s *Server) activeLogBases() map[string]struct{} {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	bases := make(map[string]struct{}, len(s.sessions))
	for _, ses := range s.sessions {
		if ses == nil {
			continue
		}
		for _, p := range []string{ses.LogPath, ses.JSONLPath} {
			if p == "" {
				continue
			}
			base := strings.TrimSuffix(p, filepath.Ext(p))
			bases[filepath.Clean(base)] = struct{}{}
		}
	}
	return bases
}

// handleLogsPurge は logs/sessions の .log/.jsonl/.txt と logs/spawn のファイルを
// すべて削除し、加えて SQLite のセッション履歴もリセットする。
// 稼働中セッションのログファイルと履歴は保護して残す。
func (s *Server) handleLogsPurge(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.logMaintenanceMu.Lock()
	logDir := s.cfg.Hub.LogDir
	activeBases := s.activeLogBases()

	sessionsRemoved := 0
	sessionsDir := filepath.Join(logDir, "sessions")
	if entries, err := os.ReadDir(sessionsDir); err == nil {
		for _, e := range entries {
			if e.IsDir() {
				continue
			}
			name := e.Name()
			if !strings.HasSuffix(name, ".log") && !strings.HasSuffix(name, ".jsonl") && !strings.HasSuffix(name, ".txt") {
				continue
			}
			full := filepath.Join(sessionsDir, name)
			base := filepath.Clean(strings.TrimSuffix(full, filepath.Ext(full)))
			if _, ok := activeBases[base]; ok {
				continue // 稼働中セッションのログは保護
			}
			if err := os.Remove(full); err == nil {
				sessionsRemoved++
			} else {
				s.logger.Debug("logs purge: remove failed", "path", full, "err", err)
			}
		}
	}

	spawnRemoved := 0
	spawnDir := filepath.Join(logDir, "spawn")
	if entries, err := os.ReadDir(spawnDir); err == nil {
		for _, e := range entries {
			if e.IsDir() {
				continue
			}
			full := filepath.Join(spawnDir, e.Name())
			if err := os.Remove(full); err == nil {
				spawnRemoved++
			} else {
				s.logger.Debug("logs purge: remove spawn failed", "path", full, "err", err)
			}
		}
	}
	s.logMaintenanceMu.Unlock()

	// SQLite のセッション履歴も削除（稼働中セッションは保護）
	storeSessions := 0
	if s.sessionStore != nil {
		if result, err := s.sessionStore.ResetHistory(s.activeSessionIDs()); err != nil {
			s.logger.Warn("logs purge: session store reset failed", "err", err)
		} else {
			storeSessions = result.Sessions
		}
	}

	s.logger.Info("logs purged", "session_files", sessionsRemoved, "spawn_files", spawnRemoved, "store_sessions", storeSessions)
	writeJSON(w, map[string]any{
		"ok":             true,
		"session_files":  sessionsRemoved,
		"spawn_files":    spawnRemoved,
		"store_sessions": storeSessions,
	})
}

// handleAttachmentsPurge は attachments 配下の全セッションフォルダを削除する。
// 稼働中セッションの添付フォルダは保護して残す。
func (s *Server) handleAttachmentsPurge(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	attachDir, err := attachmentsDir()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", "home dir unavailable")
		return
	}

	// 稼働中セッションIDをフォルダ名（"%d"）の集合に変換して保護対象とする。
	active := make(map[string]struct{})
	for _, id := range s.activeSessionIDs() {
		active[strconv.Itoa(id)] = struct{}{}
	}

	entries, err := os.ReadDir(attachDir)
	if err != nil {
		if os.IsNotExist(err) {
			writeJSON(w, map[string]any{"ok": true, "folders": 0})
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "readdir_failed", errorDetail("readdir failed", err))
		return
	}
	removed := 0
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		if _, ok := active[e.Name()]; ok {
			continue // 稼働中セッションの添付は保護
		}
		sub := filepath.Join(attachDir, e.Name())
		if err := os.RemoveAll(sub); err == nil {
			removed++
		} else {
			s.logger.Debug("attachments purge: remove failed", "path", sub, "err", err)
		}
	}
	s.logger.Info("attachments purged", "folders", removed)
	writeJSON(w, map[string]any{"ok": true, "folders": removed})
}

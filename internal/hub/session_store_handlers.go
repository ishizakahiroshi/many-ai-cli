package hub

import (
	"net/http"
	"strconv"
	"strings"
)

func (s *Server) handleSessionChat(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "messages": []any{}})
		return
	}
	q := r.URL.Query()
	limit, _ := strconv.Atoi(q.Get("limit"))
	var messages any
	var err error
	if rawDBID := strings.TrimSpace(q.Get("session_db_id")); rawDBID != "" {
		dbID, parseErr := strconv.ParseInt(rawDBID, 10, 64)
		if parseErr != nil || dbID <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_db_id")
			return
		}
		messages, err = s.sessionStore.ChatMessagesBySessionID(dbID, limit)
	} else {
		id, parseErr := strconv.Atoi(q.Get("session_id"))
		if parseErr != nil || id <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
			return
		}
		messages, err = s.sessionStore.ChatMessagesByLiveSession(id, limit)
	}
	if err != nil {
		s.logger.Warn("session chat restore failed", "session_id", q.Get("session_id"), "session_db_id", q.Get("session_db_id"), "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_chat_failed", "failed to restore chat")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "messages": messages})
}

func (s *Server) handleSessionSearch(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	q := strings.TrimSpace(r.URL.Query().Get("q"))
	if q == "" {
		writeJSON(w, map[string]any{"ok": true, "results": []any{}})
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "results": []any{}})
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	results, err := s.sessionStore.SearchMessages(q, limit)
	if err != nil {
		s.logger.Warn("session search failed", "query", q, "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_search_failed", "failed to search sessions")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "results": results})
}

// handleSessionHistory lists persisted sessions, including sessions no longer
// attached to this Hub. Transcript contents stay on the existing endpoints.
func (s *Server) handleSessionHistory(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "sessions": []any{}})
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	items, err := s.sessionStore.ListSessions(limit, true)
	if err != nil {
		s.logger.Warn("session history list failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_history_failed", "failed to list saved sessions")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "sessions": items})
}

func (s *Server) handleSessionStoreReset(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "result": map[string]any{}})
		return
	}
	activeIDs := s.activeSessionIDs()
	result, err := s.sessionStore.ResetHistory(activeIDs)
	filePending := s.sessionStore.FileResetPending()
	if filePending {
		s.logger.Info("session store file reset scheduled; db file will be recreated at next hub start")
	}
	if err != nil {
		s.logger.Warn("session store reset failed", "err", err, "file_reset_scheduled", filePending)
		writeJSONError(w, http.StatusInternalServerError, "session_store_reset_failed", "failed to reset saved session history")
		return
	}
	s.logger.Info("session store reset", "sessions", result.Sessions, "messages", result.Messages, "events", result.Events, "preserved_sessions", result.Preserved)
	writeJSON(w, map[string]any{"ok": true, "result": result, "file_reset_scheduled": filePending})
}

// handleSessionStorePruneTranscriptNoise is an explicit maintenance action.
// It only removes legacy PTY-derived noise rows for Claude/Codex; it never
// touches provider transcript files or user/approval/attachment messages.
func (s *Server) handleSessionStorePruneTranscriptNoise(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "deleted_messages": 0})
		return
	}
	deleted, err := s.sessionStore.PruneTranscriptNoise()
	if err != nil {
		s.logger.Warn("transcript noise prune failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "transcript_noise_prune_failed", "failed to prune transcript noise")
		return
	}
	s.logger.Info("transcript noise pruned", "deleted_messages", deleted)
	writeJSON(w, map[string]any{"ok": true, "deleted_messages": deleted})
}

func (s *Server) activeSessionIDs() []int {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ids := make([]int, 0, len(s.sessions))
	for id, ses := range s.sessions {
		if ses != nil {
			ids = append(ids, id)
		}
	}
	return ids
}

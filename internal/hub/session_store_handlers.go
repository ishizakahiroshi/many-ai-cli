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
	id, _ := strconv.Atoi(r.URL.Query().Get("session_id"))
	if id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	messages, err := s.sessionStore.ChatMessagesByLiveSession(id, limit)
	if err != nil {
		s.logger.Warn("session chat restore failed", "session_id", id, "err", err)
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
	if err != nil {
		s.logger.Warn("session store reset failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_store_reset_failed", "failed to reset saved session history")
		return
	}
	s.logger.Info("session store reset", "sessions", result.Sessions, "messages", result.Messages, "events", result.Events, "preserved_sessions", result.Preserved)
	writeJSON(w, map[string]any{"ok": true, "result": result})
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

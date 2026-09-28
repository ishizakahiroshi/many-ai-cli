package hub

import (
	"net/http"
	"strings"
	"time"
)

func (s *Server) memoStoreReady(w http.ResponseWriter) bool {
	if s.memos == nil || s.memos.loadErr != nil {
		writeJSONError(w, http.StatusServiceUnavailable, "memo_store_unavailable", "memo storage is unavailable")
		return false
	}
	return true
}

// handleMemos は GET/POST /api/memos と PATCH/DELETE /api/memos/<id> を捌く。
// routine_handlers.go の handleRoutines と同じ形（1 ハンドラで両ルートを吸収し、
// パスの有無で操作を振り分ける）。
func (s *Server) handleMemos(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost, http.MethodPatch, http.MethodDelete) || !s.memoStoreReady(w) {
		return
	}
	id := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/memos"), "/")
	switch r.Method {
	case http.MethodGet:
		if id != "" {
			writeJSONError(w, 404, "not_found", "memo route not found")
			return
		}
		m := s.memos
		m.mu.Lock()
		list := append([]memo{}, m.data.Memos...)
		m.mu.Unlock()
		writeJSON(w, map[string]any{"memos": list})
	case http.MethodPost:
		if id != "" {
			writeJSONError(w, 405, "method_not_allowed", "unsupported memo operation")
			return
		}
		s.handleMemoCreate(w, r)
	case http.MethodPatch:
		if id == "" {
			writeJSONError(w, 405, "method_not_allowed", "unsupported memo operation")
			return
		}
		s.handleMemoUpdate(w, r, id)
	case http.MethodDelete:
		if id == "" {
			writeJSONError(w, 405, "method_not_allowed", "unsupported memo operation")
			return
		}
		s.handleMemoDelete(w, id)
	}
}

func (s *Server) handleMemoCreate(w http.ResponseWriter, r *http.Request) {
	var body struct {
		Text      string `json:"text"`
		SessionID int    `json:"session_id,omitempty"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	text := strings.TrimSpace(body.Text)
	if text == "" || len(text) > memoTextMaxLen {
		writeJSONError(w, 400, "bad_request", "text must contain 1 to 4000 bytes")
		return
	}
	// project はここで Hub 側が決める。client から project を直接受け取らない。
	project := s.projectForSession(body.SessionID)
	m := s.memos
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.data.Memos) >= memoMaxCount {
		writeJSONError(w, 400, "memo_limit", "maximum of 500 saved memos reached")
		return
	}
	id, err := memoID()
	if err != nil {
		writeJSONError(w, 500, "memo_operation_failed", "memo operation could not be saved")
		return
	}
	now := memoTime(time.Now())
	item := memo{ID: id, Text: text, Project: project, CreatedAt: now, UpdatedAt: now}
	next := m.copyLocked()
	next.Memos = append(next.Memos, item)
	if err := m.commitLocked(next); err != nil {
		writeJSONError(w, 500, "memo_operation_failed", "memo operation could not be saved")
		return
	}
	writeJSON(w, map[string]any{"memo": item})
}

func (s *Server) handleMemoUpdate(w http.ResponseWriter, r *http.Request, id string) {
	var body struct {
		Text *string `json:"text,omitempty"`
		Done *bool   `json:"done,omitempty"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Text != nil {
		trimmed := strings.TrimSpace(*body.Text)
		if trimmed == "" || len(trimmed) > memoTextMaxLen {
			writeJSONError(w, 400, "bad_request", "text must contain 1 to 4000 bytes")
			return
		}
		body.Text = &trimmed
	}
	m := s.memos
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	idx := -1
	for i := range next.Memos {
		if next.Memos[i].ID == id {
			idx = i
			break
		}
	}
	if idx < 0 {
		writeJSONError(w, 404, "not_found", "memo not found")
		return
	}
	now := time.Now()
	if body.Text != nil {
		next.Memos[idx].Text = *body.Text
	}
	if body.Done != nil {
		if *body.Done && !next.Memos[idx].Done {
			next.Memos[idx].DoneAt = memoTime(now)
		} else if !*body.Done {
			next.Memos[idx].DoneAt = ""
		}
		next.Memos[idx].Done = *body.Done
	}
	next.Memos[idx].UpdatedAt = memoTime(now)
	if err := m.commitLocked(next); err != nil {
		writeJSONError(w, 500, "memo_operation_failed", "memo operation could not be saved")
		return
	}
	writeJSON(w, map[string]any{"memo": next.Memos[idx]})
}

func (s *Server) handleMemoDelete(w http.ResponseWriter, id string) {
	m := s.memos
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	idx := -1
	for i := range next.Memos {
		if next.Memos[i].ID == id {
			idx = i
			break
		}
	}
	if idx < 0 {
		writeJSONError(w, 404, "not_found", "memo not found")
		return
	}
	next.Memos = append(next.Memos[:idx], next.Memos[idx+1:]...)
	if err := m.commitLocked(next); err != nil {
		writeJSONError(w, 500, "memo_operation_failed", "memo operation could not be saved")
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

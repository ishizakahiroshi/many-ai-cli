package hub

import (
	"errors"
	"net/http"
	"strings"
	"time"
)

func (s *Server) routineStoreReady(w http.ResponseWriter) bool {
	if s.routines == nil || s.routines.loadErr != nil {
		writeJSONError(w, http.StatusServiceUnavailable, "routine_store_unavailable", "routine storage is unavailable")
		return false
	}
	return true
}

func (s *Server) handleRoutines(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost, http.MethodPut, http.MethodDelete) || !s.routineStoreReady(w) {
		return
	}
	path := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/routines"), "/")
	parts := strings.Split(path, "/")
	if len(parts) == 2 && parts[1] == "runs" && r.Method == http.MethodPost {
		var body struct {
			RequestID string `json:"request_id"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if strings.TrimSpace(body.RequestID) == "" || len(body.RequestID) > 128 {
			writeJSONError(w, 400, "bad_request", "request_id is required (max 128 bytes)")
			return
		}
		run, existing, err := s.startRoutine(parts[0], body.RequestID, "manual", time.Now())
		if err != nil {
			routineHTTPError(w, err)
			return
		}
		writeJSON(w, map[string]any{"run": run, "existing": existing})
		return
	}
	if len(parts) > 1 {
		writeJSONError(w, 404, "not_found", "routine route not found")
		return
	}
	m := s.routines
	if r.Method == http.MethodGet {
		m.mu.Lock()
		list := append([]routine{}, m.data.Routines...)
		m.mu.Unlock()
		if path != "" {
			for _, item := range list {
				if item.ID == path {
					writeJSON(w, map[string]any{"routine": item})
					return
				}
			}
			writeJSONError(w, 404, "not_found", "routine not found")
			return
		}
		writeJSON(w, map[string]any{"routines": list})
		return
	}
	if (r.Method == http.MethodPost && path != "") || (r.Method != http.MethodPost && path == "") {
		writeJSONError(w, 405, "method_not_allowed", "unsupported routine operation")
		return
	}
	var item routine
	if r.Method != http.MethodDelete {
		if !decodeJSON(w, r, &item) {
			return
		}
		if err := validateRoutine(&item); err != nil {
			writeJSONError(w, 400, "bad_request", err.Error())
			return
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	next := m.copyLocked()
	idx := -1
	for i := range next.Routines {
		if next.Routines[i].ID == path {
			idx = i
			break
		}
	}
	if r.Method != http.MethodPost && idx < 0 {
		writeJSONError(w, 404, "not_found", "routine not found")
		return
	}
	now := time.Now()
	if r.Method == http.MethodDelete {
		for _, run := range next.Runs {
			if run.RoutineID == path && routineActive(run.Status) {
				writeJSONError(w, 409, "routine_running", "wait for the current run before deleting this routine")
				return
			}
		}
		next.Routines = append(next.Routines[:idx], next.Routines[idx+1:]...)
	} else {
		if idx >= 0 && item.UpdatedAt != "" && item.UpdatedAt != next.Routines[idx].UpdatedAt {
			writeJSONError(w, 409, "routine_changed", "routine changed; reload before saving")
			return
		}
		item.UpdatedAt = routineTime(now)
		item.NextRunAt = ""
		if item.Enabled {
			due, _ := nextRoutineTime(item.Schedule, now)
			if !due.IsZero() {
				item.NextRunAt = routineTime(due)
			}
		}
		if idx < 0 {
			if len(next.Routines) >= 200 {
				writeJSONError(w, 409, "routine_limit", "maximum of 200 saved routines reached")
				return
			}
			id, err := routineID()
			if err != nil {
				routineHTTPError(w, err)
				return
			}
			item.ID = id
			item.CreatedAt = item.UpdatedAt
			next.Routines = append(next.Routines, item)
		} else {
			item.ID = path
			item.CreatedAt = next.Routines[idx].CreatedAt
			next.Routines[idx] = item
		}
	}
	if err := m.commitLocked(next); err != nil {
		routineHTTPError(w, err)
		return
	}
	if r.Method == http.MethodDelete {
		writeJSON(w, map[string]bool{"ok": true})
		return
	}
	writeJSON(w, map[string]any{"routine": item})
}

var errRoutineMissing = errors.New("routine not found")

func routineHTTPError(w http.ResponseWriter, err error) {
	if errors.Is(err, errRoutineMissing) {
		writeJSONError(w, 404, "not_found", err.Error())
		return
	}
	// Storage/process details can contain local paths or credentials; keep the wire error fixed.
	writeJSONError(w, 500, "routine_operation_failed", "routine operation could not be saved or started")
}

func (s *Server) handleRoutineRuns(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) || !s.routineStoreReady(w) {
		return
	}
	id := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/routine-runs"), "/")
	m := s.routines
	m.mu.Lock()
	defer m.mu.Unlock()
	if id != "" {
		for _, run := range m.data.Runs {
			if run.ID == id {
				writeJSON(w, map[string]any{"run": run})
				return
			}
		}
		writeJSONError(w, 404, "not_found", "routine run not found")
		return
	}
	runs := []routineRun{}
	filter := r.URL.Query().Get("routine_id")
	for i := len(m.data.Runs) - 1; i >= 0; i-- {
		if filter == "" || m.data.Runs[i].RoutineID == filter {
			runs = append(runs, m.data.Runs[i])
		}
		if len(runs) >= 200 {
			break
		}
	}
	writeJSON(w, map[string]any{"runs": runs})
}

package hub

// handoff_handler.go は看板 jsonl から組んだ引き継ぎ md を Web UI へ渡す HTTP
// 経路。子 plan: docs/local/plan_session-handoff-board_c5_handoff-md.md
// 内部 C1（プレビュー）・C3（一覧）。
//
// 起動そのもの（新しいセッションを立てる）はここでは行わない。ブラウザは
// このファイルが返した markdown を、既存の C4 の入口である
// POST /api/spawn（initial_prompt・handoff_from 付き）へそのまま渡す。
// 起動処理を複製せず、検証済みの 1 本（handleSpawn）だけに通す設計判断
// （親 plan 「後継は子ではなく新しい親」・CLAUDE/coding.md 根本原因優先）。
import (
	"net/http"
	"strconv"
	"strings"
)

// handleHandoffList handles GET /api/handoff: every session with a handoff
// record on disk, newest first (子 plan 内部 C3 の「一覧」導線)。
func (s *Server) handleHandoffList(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	entries, err := s.handoffListEntries()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "handoff_list_error", errorDetail("handoff list error", err))
		return
	}
	s.sessionsMu.Lock()
	for i := range entries {
		_, entries[i].Live = s.sessions[entries[i].SessionID]
	}
	s.sessionsMu.Unlock()
	writeJSON(w, map[string]any{"ok": true, "entries": entries})
}

// handleHandoffItem handles GET /api/handoff/{id}: the rendered preview for
// one session (子 plan 内部 C1). A session with no handoff jsonl on disk
// (never recorded, or already pruned) answers ok:true, exists:false rather
// than 404 — "nothing to show" is an ordinary state here, not an error.
func (s *Server) handleHandoffItem(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	idStr := strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/handoff/"), "/")
	id, err := strconv.Atoi(idStr)
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session id")
		return
	}
	preview, err := s.handoffPreviewFor(id)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "handoff_render_error", errorDetail("handoff render error", err))
		return
	}
	s.sessionsMu.Lock()
	_, preview.Live = s.sessions[id]
	s.sessionsMu.Unlock()
	preview.CandidateProviders = handoffCandidateProviders(preview.Provider)
	writeJSON(w, preview)
}

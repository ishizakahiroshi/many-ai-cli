package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	"many-ai-cli/internal/handoff"
)

// TestHandoffCandidateProvidersExcludesSource is the C2/C3 completion
// criterion "同一 provider の別 subscription profile は候補に出さない",
// implemented by never offering the source session's own provider at all.
func TestHandoffCandidateProvidersExcludesSource(t *testing.T) {
	got := handoffCandidateProviders("claude")
	for _, p := range got {
		if p == "claude" {
			t.Fatalf("candidate list must not include the source provider: %v", got)
		}
	}
	want := map[string]bool{"codex": true, "copilot": true, "cursor-agent": true, "opencode": true, "grok": true}
	for _, p := range got {
		delete(want, p)
	}
	if len(want) != 0 {
		t.Fatalf("candidate list missing providers: %v (got %v)", want, got)
	}
}

// TestHandleHandoffItemNotExists is the C1 completion criterion's negative
// case: a session with no handoff jsonl on disk answers exists:false, not an
// error.
func TestHandleHandoffItemNotExists(t *testing.T) {
	s, _ := subsTestServer(t)

	w := httptest.NewRecorder()
	s.handleHandoffItem(w, subsRequest(t, http.MethodGet, "/api/handoff/999", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200: %s", w.Code, w.Body.String())
	}
	var got struct {
		OK     bool `json:"ok"`
		Exists bool `json:"exists"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if !got.OK || got.Exists {
		t.Fatalf("got %+v, want ok=true exists=false", got)
	}
}

// TestHandleHandoffItemRendersMarkdown is the C1 completion criterion: a
// session with a recorded board renders the six-section markdown and writes
// it to the *_handoff.md sibling, and the candidate provider list excludes
// the session's own provider.
func TestHandleHandoffItemRendersMarkdown(t *testing.T) {
	s, _ := subsTestServer(t)

	const sessionID = 501
	if err := handoff.Append(sessionID, handoff.Record{Kind: handoff.KindSessionStart, Provider: "codex", CWD: `C:\work\sample-repo`}); err != nil {
		t.Fatalf("Append: %v", err)
	}
	if err := handoff.Append(sessionID, handoff.Record{Kind: handoff.KindDone, Text: "[success] did the thing"}); err != nil {
		t.Fatalf("Append: %v", err)
	}

	w := httptest.NewRecorder()
	s.handleHandoffItem(w, subsRequest(t, http.MethodGet, "/api/handoff/501", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d: %s", w.Code, w.Body.String())
	}
	var got struct {
		OK                 bool     `json:"ok"`
		Exists             bool     `json:"exists"`
		Provider           string   `json:"provider"`
		Markdown           string   `json:"markdown"`
		CandidateProviders []string `json:"candidate_providers"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if !got.OK || !got.Exists || got.Provider != "codex" {
		t.Fatalf("got %+v", got)
	}
	if !strings.Contains(got.Markdown, "セッションの素性") || !strings.Contains(got.Markdown, "[success] did the thing") {
		t.Fatalf("markdown missing expected sections: %q", got.Markdown)
	}
	for _, p := range got.CandidateProviders {
		if p == "codex" {
			t.Fatalf("candidate_providers must exclude the session's own provider: %v", got.CandidateProviders)
		}
	}

	path, err := handoff.RenderedPathFor(sessionID)
	if err != nil {
		t.Fatalf("RenderedPathFor: %v", err)
	}
	if _, statErr := os.Stat(path); statErr != nil {
		t.Fatalf("expected rendered markdown file at %s: %v", path, statErr)
	}
}

// TestHandleHandoffListNewestFirst is the C3 completion criterion: the list
// endpoint reads the handoff directory directly (not s.sessions), so it
// answers even for sessions this Server instance never registered live —
// the same guarantee a Hub restart needs.
func TestHandleHandoffListNewestFirst(t *testing.T) {
	s, _ := subsTestServer(t)

	for _, id := range []int{10, 30, 20} {
		if err := handoff.Append(id, handoff.Record{Kind: handoff.KindSessionStart, Provider: "claude"}); err != nil {
			t.Fatalf("Append(%d): %v", id, err)
		}
	}

	w := httptest.NewRecorder()
	s.handleHandoffList(w, subsRequest(t, http.MethodGet, "/api/handoff", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d: %s", w.Code, w.Body.String())
	}
	var got struct {
		OK      bool `json:"ok"`
		Entries []struct {
			SessionID int  `json:"session_id"`
			Live      bool `json:"live"`
		} `json:"entries"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if !got.OK || len(got.Entries) != 3 {
		t.Fatalf("got %+v, want 3 entries", got)
	}
	if got.Entries[0].SessionID != 30 || got.Entries[1].SessionID != 20 || got.Entries[2].SessionID != 10 {
		t.Fatalf("entries not newest-first: %+v", got.Entries)
	}
	for _, e := range got.Entries {
		if e.Live {
			t.Fatalf("session %d marked live, but this Server never registered it: %+v", e.SessionID, got.Entries)
		}
	}
}

// TestHandleSpawnRejectsNegativeHandoffFrom guards the new field's
// validation: a negative handoff_from can only be a caller bug or tampering,
// never a real predecessor session ID.
func TestHandleSpawnRejectsNegativeHandoffFrom(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":     "opencode",
		"cwd":          s.hubCWD,
		"handoff_from": -1,
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, body = %s, want 400", w.Code, w.Body.String())
	}
}

// TestHandleSpawnHandoffFromStoredInPending mirrors
// TestHandleSpawnInitialPromptStoredInPending: a handoff-launched /api/spawn
// (initial_prompt + handoff_from together, as the browser sends them) must
// carry handoff_from through to the pending entry wrapperLoop's registration
// handshake reads, so recordHandoffSessionStart can tag the successor's own
// session_start with its predecessor (子 plan 内部 C3).
func TestHandleSpawnHandoffFromStoredInPending(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":        "opencode",
		"cwd":             s.hubCWD,
		"permission_mode": "bypassPermissions",
		"initial_prompt":  "resume from the handoff",
		"handoff_from":    42,
	}))
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "risk_confirmation_required") {
		t.Fatalf("code = %d, body = %s, want 400 risk_confirmation_required", w.Code, w.Body.String())
	}
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	found := false
	for _, meta := range s.orchestration.pending {
		if meta.HandoffFrom == 42 {
			found = true
		}
	}
	if !found {
		t.Fatalf("no pending entry carries handoff_from=42; pending = %+v", s.orchestration.pending)
	}
}

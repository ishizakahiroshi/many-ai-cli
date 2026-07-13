package hub

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"many-ai-cli/internal/sessionstore"
)

func TestSessionCardMetaPersistsAcrossReattach(t *testing.T) {
	store, err := sessionstore.OpenForLogDir(filepath.Join(t.TempDir(), "logs"))
	if err != nil {
		t.Fatalf("open store: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })
	start := sessionstore.SessionStart{
		LiveSessionID: 1, Provider: "claude", Label: "spawn-label", State: "running",
		StartedAt: time.Now().Format(time.RFC3339), JSONLPath: filepath.Join(t.TempDir(), "one.jsonl"),
	}
	if _, err := store.StartSession(start); err != nil {
		t.Fatalf("start session: %v", err)
	}
	want := sessionstore.SessionCardMeta{Label: "PR review", Pinned: true, Color: "purple", Note: "wait for CI", AutoTitle: "review the PR"}
	if err := store.UpdateSessionCardMeta(1, want); err != nil {
		t.Fatalf("save card meta: %v", err)
	}
	// The same JSONL path is used by a wrapper reattach. Its register label must
	// not overwrite the name the user chose in the card menu.
	start.Label = "wrapper-label"
	if _, err := store.StartSession(start); err != nil {
		t.Fatalf("reattach session: %v", err)
	}
	got, err := store.SessionCardMetaByLiveSession(1)
	if err != nil {
		t.Fatalf("load card meta: %v", err)
	}
	if got != want {
		t.Fatalf("card meta = %#v, want %#v", got, want)
	}
}

func TestHandleSessionMetaAcceptsExplicitClears(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "test-token"
	s.sessions[1] = &session{ID: 1, Provider: "codex", State: "running", Label: "before", Color: "red", Note: "note"}
	req := httptest.NewRequest(http.MethodPatch, "http://127.0.0.1/api/session/1/meta?token=test-token", bytes.NewBufferString(`{"label":"","color":"","note":"","pinned":true}`))
	req.RemoteAddr = "127.0.0.1:32100"
	req.Header.Set("Content-Type", "application/json")
	rr := httptest.NewRecorder()
	s.handleSessionMetaAPI(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", rr.Code, rr.Body.String())
	}
	got := s.sessions[1]
	if got.Label != "" || got.Color != "" || got.Note != "" || !got.Pinned {
		t.Fatalf("patched session = %#v", got)
	}
}

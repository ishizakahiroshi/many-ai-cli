package hub

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"runtime"
	"testing"

	"many-ai-cli/internal/config"
)

func TestHandleWhisperStatusRequiresToken(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodGet, "/api/whisper/status", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleWhisperStatus(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusUnauthorized)
	}
}

func TestEnsureManagedWhisperNotInstalled(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.cfg.Voice.Whisper = config.VoiceWhisperConfig{
		Managed:        true,
		Model:          "tiny-q5_1",
		Language:       "ja",
		TimeoutSeconds: 5,
	}
	req := httptest.NewRequest(http.MethodPost, "/api/voice/transcribe?token=tok", bytes.NewReader([]byte("RIFF")))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleVoiceTranscribe(w, req)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d body=%s", w.Code, http.StatusBadRequest, w.Body.String())
	}
	want := "whisper_not_installed"
	if runtime.GOOS != "windows" {
		// managed Whisper is Windows-only; other platforms report it as unsupported.
		want = "unsupported_platform"
	}
	if !bytes.Contains(w.Body.Bytes(), []byte(want)) {
		t.Fatalf("body missing %s: %s", want, w.Body.String())
	}
}

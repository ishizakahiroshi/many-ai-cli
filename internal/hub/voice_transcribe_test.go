package hub

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"many-ai-cli/internal/config"
)

func TestHandleVoiceTranscribeNotConfigured(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	req := httptest.NewRequest(http.MethodPost, "/api/voice/transcribe?token=tok", bytes.NewReader([]byte("RIFF")))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleVoiceTranscribe(w, req)
	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want %d body=%s", w.Code, http.StatusBadRequest, w.Body.String())
	}
	if !bytes.Contains(w.Body.Bytes(), []byte("whisper_not_configured")) {
		t.Fatalf("body missing whisper_not_configured: %s", w.Body.String())
	}
}

func TestHandleVoiceTranscribeOpenAICompatiblePath(t *testing.T) {
	var gotPath, gotLanguage, gotModel string
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		if err := r.ParseMultipartForm(1024 * 1024); err != nil {
			t.Fatalf("ParseMultipartForm: %v", err)
		}
		gotLanguage = r.FormValue("language")
		gotModel = r.FormValue("model")
		file, _, err := r.FormFile("file")
		if err != nil {
			t.Fatalf("FormFile: %v", err)
		}
		defer file.Close()
		body, _ := io.ReadAll(file)
		if string(body) != "RIFF" {
			t.Fatalf("audio body = %q", string(body))
		}
		_ = json.NewEncoder(w).Encode(map[string]string{"text": "こんにちは"})
	}))
	defer ts.Close()

	s := newSecTestServer(t, t.TempDir())
	s.cfg.Voice.Whisper = config.VoiceWhisperConfig{
		ServerURL:      ts.URL,
		Language:       "ja",
		TimeoutSeconds: 5,
	}
	req := httptest.NewRequest(http.MethodPost, "/api/voice/transcribe?token=tok", bytes.NewReader([]byte("RIFF")))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleVoiceTranscribe(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200 body=%s", w.Code, w.Body.String())
	}
	if gotPath != "/v1/audio/transcriptions" {
		t.Fatalf("path = %q", gotPath)
	}
	if gotLanguage != "ja" {
		t.Fatalf("language = %q", gotLanguage)
	}
	if gotModel != "whisper-1" {
		t.Fatalf("model = %q", gotModel)
	}
	if !bytes.Contains(w.Body.Bytes(), []byte("こんにちは")) {
		t.Fatalf("body missing text: %s", w.Body.String())
	}
}

func TestTranscribeWithWhisperFallbacksToInference(t *testing.T) {
	var sawOpenAI, sawInference bool
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/audio/transcriptions":
			sawOpenAI = true
			http.NotFound(w, r)
		case "/inference":
			sawInference = true
			_ = json.NewEncoder(w).Encode(map[string]string{"text": "fallback ok"})
		default:
			t.Fatalf("unexpected path %s", r.URL.Path)
		}
	}))
	defer ts.Close()

	text, err := transcribeWithWhisper(t.Context(), config.VoiceWhisperConfig{
		ServerURL:      ts.URL,
		Language:       "auto",
		TimeoutSeconds: 5,
	}, []byte("RIFF"))
	if err != nil {
		t.Fatalf("transcribeWithWhisper: %v", err)
	}
	if text != "fallback ok" {
		t.Fatalf("text = %q", text)
	}
	if !sawOpenAI || !sawInference {
		t.Fatalf("fallback paths sawOpenAI=%v sawInference=%v", sawOpenAI, sawInference)
	}
}

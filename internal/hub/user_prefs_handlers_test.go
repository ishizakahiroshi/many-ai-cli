package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func prefsAuthReq(method, path string, body []byte, contentType string) *http.Request {
	req := httptest.NewRequest(method, path, bytes.NewReader(body))
	req.Host = testHubHost
	req.RemoteAddr = testLoopbackAddr
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	return req
}

func TestNotifySoundCustomPutRejectsNonAudioDespiteAudioHeader(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"

	body := []byte("<html><script>alert(1)</script></html>")
	req := prefsAuthReq(http.MethodPut, "/api/user-prefs/notify-sound-custom?token=tok", body, "audio/mpeg")
	w := httptest.NewRecorder()
	s.handleUserPrefsNotifySoundCustom(w, req)
	if w.Code != http.StatusUnsupportedMediaType {
		t.Fatalf("status = %d body=%s, want 415", w.Code, w.Body.String())
	}
	path, err := notifySoundCustomPath()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("rejected body must not be written: err=%v", err)
	}
}

func TestNotifySoundCustomPutAcceptsSniffedWAV(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"
	if err := os.MkdirAll(filepath.Join(home, ".many-ai-cli"), 0o700); err != nil {
		t.Fatal(err)
	}

	// Minimal RIFF/WAVE header; DetectContentType returns audio/wave.
	wav := []byte("RIFFxxxxWAVEfmt ")
	req := prefsAuthReq(http.MethodPut, "/api/user-prefs/notify-sound-custom?token=tok", wav, "application/octet-stream")
	w := httptest.NewRecorder()
	s.handleUserPrefsNotifySoundCustom(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body=%s, want 200", w.Code, w.Body.String())
	}
	if got := s.cfg.UserPrefs.NotifySound.CustomMime; !strings.HasPrefix(got, "audio/") {
		t.Fatalf("CustomMime = %q, want audio/*", got)
	}
}

func TestAvatarUploadPutRejectsNonImage(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"

	body := []byte(`<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>`)
	req := prefsAuthReq(http.MethodPut, "/api/user-prefs/avatar?token=tok", body, "image/svg+xml")
	w := httptest.NewRecorder()
	s.handleUserPrefsAvatarUpload(w, req)
	if w.Code != http.StatusUnsupportedMediaType {
		t.Fatalf("status = %d body=%s, want 415", w.Code, w.Body.String())
	}
}

func TestAvatarUploadPutAcceptsPNG(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"
	if err := os.MkdirAll(filepath.Join(home, ".many-ai-cli"), 0o700); err != nil {
		t.Fatal(err)
	}

	png := []byte("\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR")
	req := prefsAuthReq(http.MethodPut, "/api/user-prefs/avatar?token=tok", png, "application/octet-stream")
	w := httptest.NewRecorder()
	s.handleUserPrefsAvatarUpload(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body=%s, want 200", w.Code, w.Body.String())
	}
	path, err := avatarUploadPath()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("avatar file missing: %v", err)
	}
	if s.cfg.UserPrefs.Avatar != path {
		t.Fatalf("Avatar pref = %q, want %q", s.cfg.UserPrefs.Avatar, path)
	}
}

func TestHandleInfoAvatarURLOmitsToken(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "super-secret-token"
	upload, err := avatarUploadPath()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(upload), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(upload, []byte("\x89PNG\r\n\x1a\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.cfg.UserPrefs.Avatar = upload

	req := prefsAuthReq(http.MethodGet, "/api/info?token=super-secret-token", nil, "")
	w := httptest.NewRecorder()
	s.handleInfo(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body=%s", w.Code, w.Body.String())
	}
	var body map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	got, _ := body["userAvatar"].(string)
	if got != "/api/avatar" {
		t.Fatalf("userAvatar = %q, want /api/avatar (no token)", got)
	}
	if strings.Contains(w.Body.String(), "super-secret-token") {
		t.Fatal("token must not appear in /api/info avatar URL")
	}
}

func TestAvatarImageAllowed(t *testing.T) {
	for _, mime := range []string{"image/png", "image/jpeg", "image/gif", "image/webp"} {
		if !avatarImageAllowed(mime) {
			t.Fatalf("%s should be allowed", mime)
		}
	}
	for _, mime := range []string{"image/svg+xml", "text/html", "text/xml", "application/octet-stream", ""} {
		if avatarImageAllowed(mime) {
			t.Fatalf("%s should be rejected", mime)
		}
	}
}

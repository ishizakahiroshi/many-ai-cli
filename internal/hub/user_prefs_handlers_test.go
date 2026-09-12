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

	"many-ai-cli/internal/config"
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

// 箱ごとの表示記憶（project_views）と最後に開いていた箱（open_project）が
// PUT → GET で往復することを固定する。
//
// PUT は UserPrefs を丸ごと置き換えるので、往復できないフィールドは「別のクライアントが
// 保存した瞬間に消える」側の壊れ方をする。画面からは保存できているように見えて、次に
// 開いたときだけ記憶が無い。
func TestUserPrefsPutRoundTripsProjectViews(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"

	body := []byte(`{"project_views":{"/src/box-alpha":{"session_id":7,"tab":"git"}},"open_project":"/src/box-alpha"}`)
	w := httptest.NewRecorder()
	s.handleUserPrefsPut(w, prefsAuthReq(http.MethodPut, "/api/user-prefs?token=tok", body, "application/json"))
	if w.Code != http.StatusOK {
		t.Fatalf("PUT status = %d body=%s, want 200", w.Code, w.Body.String())
	}

	w = httptest.NewRecorder()
	s.handleUserPrefsGet(w, prefsAuthReq(http.MethodGet, "/api/user-prefs?token=tok", nil, ""))
	if w.Code != http.StatusOK {
		t.Fatalf("GET status = %d body=%s, want 200", w.Code, w.Body.String())
	}
	var got config.UserPrefs
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatalf("GET body is not UserPrefs JSON: %v (%s)", err, w.Body.String())
	}
	if got.OpenProject != "/src/box-alpha" {
		t.Fatalf("OpenProject = %q, want /src/box-alpha", got.OpenProject)
	}
	view, ok := got.ProjectViews["/src/box-alpha"]
	if !ok {
		t.Fatalf("ProjectViews lost the entry: %#v", got.ProjectViews)
	}
	if view.SessionID != 7 || view.Tab != "git" {
		t.Fatalf("ProjectViews entry = %#v, want {SessionID:7 Tab:git}", view)
	}
}

// Clone が map を共有すると、/api/info 等が持ち出したスナップショットの書き換えが
// 動いているサーバー設定へ波及する。
func TestUserPrefsCloneDoesNotShareProjectViews(t *testing.T) {
	var prefs config.UserPrefs
	prefs.ProjectViews = map[string]config.UserPrefsProjectView{
		"/src/box-alpha": {SessionID: 7, Tab: "git"},
	}
	clone := prefs.Clone()
	clone.ProjectViews["/src/box-alpha"] = config.UserPrefsProjectView{SessionID: 99, Tab: "chat"}
	clone.ProjectViews["/src/box-bravo"] = config.UserPrefsProjectView{SessionID: 1, Tab: "terminal"}

	if got := prefs.ProjectViews["/src/box-alpha"]; got.SessionID != 7 || got.Tab != "git" {
		t.Fatalf("元の entry が複製側の書き換えで変わった: %#v", got)
	}
	if len(prefs.ProjectViews) != 1 {
		t.Fatalf("元の map へ複製側の追加が波及した: %#v", prefs.ProjectViews)
	}
}

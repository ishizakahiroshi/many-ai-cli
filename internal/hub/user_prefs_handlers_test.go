package hub

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"gopkg.in/yaml.v3"
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

func TestUserPrefsTemplateConcurrencyAndEmptyList(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.UserPrefs.Templates = []config.UserPrefsTemplate{{Body: "Original synthetic instruction"}}
	getVersion := func() string {
		w := httptest.NewRecorder()
		s.handleUserPrefsGet(w, prefsAuthReq(http.MethodGet, "/api/user-prefs", nil, ""))
		version := w.Header().Get("X-Template-Version")
		if len(version) != 64 {
			t.Fatalf("missing template version: %q", version)
		}
		return version
	}
	put := func(body, policy, version string) *httptest.ResponseRecorder {
		req := prefsAuthReq(http.MethodPut, "/api/user-prefs", []byte(body), "application/json")
		req.Header.Set("X-Template-Write", policy)
		req.Header.Set("X-Template-Version", version)
		w := httptest.NewRecorder()
		s.handleUserPrefsPut(w, req)
		return w
	}
	original := getVersion()
	updated := put(`{"templates":[{"body":"Updated on desktop"}]}`, "compare", original)
	if updated.Code != http.StatusOK {
		t.Fatalf("first edit failed: %d %s", updated.Code, updated.Body.String())
	}
	newVersion := getVersion()
	if newVersion == original || updated.Header().Get("X-Template-Version") != newVersion {
		t.Fatal("accepted edit did not advance the template version")
	}
	// Another device fetched the old list before the first edit. Saving an
	// unrelated setting must preserve the live list under the server lock.
	unrelated := put(`{"templates":[{"body":"Stale phone cache"}],"open_project":"/synthetic/project"}`, "preserve", original)
	if unrelated.Code != http.StatusOK || s.cfg.UserPrefs.Templates[0].Body != "Updated on desktop" || s.cfg.UserPrefs.OpenProject != "/synthetic/project" {
		t.Fatalf("unrelated save lost shared data or settings: %d %#v", unrelated.Code, s.cfg.UserPrefs)
	}
	stale := put(`{"templates":[{"body":"Conflicting phone edit"}]}`, "compare", original)
	if stale.Code != http.StatusConflict || s.cfg.UserPrefs.Templates[0].Body != "Updated on desktop" {
		t.Fatalf("stale edit overwrote live templates: %d", stale.Code)
	}
	if getVersion() != newVersion {
		t.Fatal("unrelated/rejected saves changed template version")
	}
	empty := put(`{"templates":[]}`, "compare", newVersion)
	if empty.Code != http.StatusOK || len(s.cfg.UserPrefs.Templates) != 0 {
		t.Fatalf("clearing the list failed: %d %#v", empty.Code, s.cfg.UserPrefs.Templates)
	}
	if getVersion() != userPrefsTemplateVersion(nil) {
		t.Fatal("empty and omitted lists disagree")
	}
	missingVersion := put(`{"templates":[{"body":"No baseline"}]}`, "compare", "")
	if missingVersion.Code != http.StatusConflict {
		t.Fatalf("compare without baseline = %d", missingVersion.Code)
	}
}

func TestUserPrefsTemplateFailedSaveDoesNotPublishVersion(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.UserPrefs.Templates = []config.UserPrefsTemplate{{Body: "Original synthetic instruction"}}
	original := userPrefsTemplateVersion(s.cfg.UserPrefs.Templates)
	// A directory at the destination makes the atomic rename fail, without
	// relying on platform-specific read-only bits or replacing global seams.
	if err := os.MkdirAll(filepath.Join(home, ".many-ai-cli", "config.yaml"), 0o700); err != nil {
		t.Fatal(err)
	}
	r := prefsAuthReq(http.MethodPut, "/api/user-prefs", []byte(`{"templates":[{"body":"Unsaved edit"}],"open_project":"/unsaved/project"}`), "application/json")
	r.Header.Set("X-Template-Write", "compare")
	r.Header.Set("X-Template-Version", original)
	w := httptest.NewRecorder()
	s.handleUserPrefsPut(w, r)
	if w.Code != http.StatusInternalServerError {
		t.Fatalf("save failure status=%d", w.Code)
	}
	if userPrefsTemplateVersion(s.cfg.UserPrefs.Templates) != original || s.cfg.UserPrefs.OpenProject != "" {
		t.Fatal("failed save published uncommitted preferences")
	}
	if w.Header().Get("X-Template-Version") != "" {
		t.Fatal("failed save advertised a committed version")
	}
}

func TestUserPrefsTemplateParallelComparePreserveAndResponseVersion(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.UserPrefs.Templates = []config.UserPrefsTemplate{{Body: "Original synthetic instruction"}}
	version := userPrefsTemplateVersion(s.cfg.UserPrefs.Templates)
	type response struct {
		compare  bool
		recorder *httptest.ResponseRecorder
	}
	responses := make(chan response, 16)
	configErrors := make(chan error, 8)
	start := make(chan struct{})
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			policy := "preserve"
			if i%2 == 0 {
				policy = "compare"
			}
			body, _ := json.Marshal(config.UserPrefs{Templates: []config.UserPrefsTemplate{{Body: fmt.Sprintf("Synthetic edit %d", i)}}, OpenProject: fmt.Sprintf("/project/%d", i)})
			r := prefsAuthReq(http.MethodPut, "/api/user-prefs", body, "application/json")
			r.Header.Set("X-Template-Write", policy)
			r.Header.Set("X-Template-Version", version)
			w := httptest.NewRecorder()
			<-start
			s.handleUserPrefsPut(w, r)
			responses <- response{compare: policy == "compare", recorder: w}
		}(i)
	}
	// Other settings endpoints mutate under cfgMu and call persistConfig after
	// releasing it. Those saves must not flush an older shared-template snapshot
	// after a compare/preserve transaction has already acknowledged its change.
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			<-start
			s.cfgMu.Lock()
			s.cfg.Input.DeferredEnterMS = 100 + i
			s.cfgMu.Unlock()
			configErrors <- s.persistConfig()
		}(i)
	}
	close(start)
	wg.Wait()
	close(responses)
	close(configErrors)
	for err := range configErrors {
		if err != nil {
			t.Fatalf("concurrent config save failed: %v", err)
		}
	}
	winners := 0
	for item := range responses {
		w := item.recorder
		if w.Code == http.StatusConflict && item.compare {
			continue
		}
		if w.Code != http.StatusOK {
			t.Fatalf("unexpected status=%d body=%s", w.Code, w.Body.String())
		}
		if item.compare {
			winners++
		}
		var saved config.UserPrefs
		if err := json.Unmarshal(w.Body.Bytes(), &saved); err != nil {
			t.Fatal(err)
		}
		if w.Header().Get("X-Template-Version") != userPrefsTemplateVersion(saved.Templates) {
			t.Fatal("response version describes a different template snapshot")
		}
	}
	if winners != 1 {
		t.Fatalf("CAS writers admitted=%d, want 1", winners)
	}
	data, err := os.ReadFile(filepath.Join(home, ".many-ai-cli", "config.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	var persisted config.Config
	if err := yaml.Unmarshal(data, &persisted); err != nil {
		t.Fatal(err)
	}
	if userPrefsTemplateVersion(persisted.UserPrefs.Templates) != userPrefsTemplateVersion(s.cfg.UserPrefs.Templates) {
		t.Fatal("saved templates differ from the published version")
	}
	if persisted.UserPrefs.OpenProject != s.cfg.UserPrefs.OpenProject {
		t.Fatal("saved preferences differ from the published snapshot")
	}
	if persisted.Input.DeferredEnterMS != s.cfg.Input.DeferredEnterMS {
		t.Fatal("saved non-preference setting differs from the published snapshot")
	}
}

// Clone が map を共有すると、/api/info 等が持ち出したスナップショットの書き換えが
// 動いているサーバー設定へ波及する。
func TestUserPrefsPutRoundTripsCustomThemes(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"

	body := []byte(`{"display":{"theme":"u-ok","custom_themes":[{"id":"u-ok","name":"濃い","mode":"dark","hue":220,"contrast":80},{"id":"dark","name":"bad"}]}}`)
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
	if got.Display.Theme != "u-ok" {
		t.Fatalf("Theme = %q, want u-ok", got.Display.Theme)
	}
	if len(got.Display.CustomThemes) != 1 || got.Display.CustomThemes[0].Name != "濃い" {
		t.Fatalf("CustomThemes = %#v", got.Display.CustomThemes)
	}
}

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

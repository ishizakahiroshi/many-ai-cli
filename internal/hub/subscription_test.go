package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// subsTestServer は HOME を差し替えた Server を返す。profile ディレクトリは
// ~/.many-ai-cli/subscriptions/ の下に作られるため、テストごとに隔離する。
func subsTestServer(t *testing.T) (*Server, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"
	return s, home
}

func subsRequest(t *testing.T, method, path string, body any) *http.Request {
	t.Helper()
	var req *http.Request
	if body == nil {
		req = httptest.NewRequest(method, path+"?token=tok", nil)
	} else {
		raw, err := json.Marshal(body)
		if err != nil {
			t.Fatal(err)
		}
		req = httptest.NewRequest(method, path+"?token=tok", strings.NewReader(string(raw)))
		req.Header.Set("Content-Type", "application/json")
	}
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	return req
}

// TestSubscriptionLaunchWithoutProfileLeavesEnvUnchanged は、profile を指定しない
// 起動の env が従来と 1 バイトも変わらないことを確認する。後方互換の中心。
func TestSubscriptionLaunchWithoutProfileLeavesEnvUnchanged(t *testing.T) {
	s, _ := subsTestServer(t)
	for _, id := range []string{"", "   "} {
		env, resolved, err := s.subscriptionLaunch("claude", id)
		if err != nil {
			t.Fatalf("subscriptionLaunch(%q) = %v", id, err)
		}
		if env != nil || resolved != nil {
			t.Fatalf("subscriptionLaunch(%q) = (%v, %#v), want (nil, nil)", id, env, resolved)
		}
	}
	base := []string{"PATH=/usr/bin", "MANY_AI_CLI=1"}
	merged := mergeEnvOverrides(base, nil)
	if len(merged) != len(base) {
		t.Fatalf("merged env = %v, want the base unchanged", merged)
	}
	for i := range base {
		if merged[i] != base[i] {
			t.Fatalf("merged[%d] = %q, want %q", i, merged[i], base[i])
		}
	}
}

func TestSubscriptionLaunchBuildsProfileEnv(t *testing.T) {
	s, home := subsTestServer(t)
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {{ID: "main", Name: "Main"}, {ID: "sub", Name: "Sub"}},
	}
	envA, a, err := s.subscriptionLaunch("claude", "main")
	if err != nil {
		t.Fatalf("subscriptionLaunch(main): %v", err)
	}
	envB, b, err := s.subscriptionLaunch("claude", "sub")
	if err != nil {
		t.Fatalf("subscriptionLaunch(sub): %v", err)
	}
	wantA := filepath.Join(home, ".many-ai-cli", "subscriptions", "claude", "main")
	if a.ProfileDir != wantA {
		t.Fatalf("profile dir = %q, want %q", a.ProfileDir, wantA)
	}
	// 起動前にディレクトリが用意されていること。
	if info, statErr := os.Stat(wantA); statErr != nil || !info.IsDir() {
		t.Fatalf("profile dir %q was not created: %v", wantA, statErr)
	}

	joinedA, joinedB := strings.Join(envA, "\n"), strings.Join(envB, "\n")
	if !strings.Contains(joinedA, subscription.ClaudeConfigDirEnv+"=") {
		t.Fatalf("env %v does not set %s", envA, subscription.ClaudeConfigDirEnv)
	}
	if !strings.Contains(joinedA, subscription.SessionEnvVar+"=main") {
		t.Fatalf("env %v does not carry the profile id back to the wrapper", envA)
	}
	if strings.Contains(joinedA, b.ProfileDir) || strings.Contains(joinedB, a.ProfileDir) {
		t.Fatalf("profile env leaked across profiles:\nA=%v\nB=%v", envA, envB)
	}

	// 同時起動を模して 2 つの env を実際に重ねても混ざらないこと。
	base := []string{"PATH=/usr/bin"}
	mergedA := mergeEnvOverrides(append([]string(nil), base...), envA)
	mergedB := mergeEnvOverrides(append([]string(nil), base...), envB)
	if strings.Contains(strings.Join(mergedA, "\n"), b.ProfileDir) {
		t.Fatalf("session A env contains session B's profile dir: %v", mergedA)
	}
	if strings.Contains(strings.Join(mergedB, "\n"), a.ProfileDir) {
		t.Fatalf("session B env contains session A's profile dir: %v", mergedB)
	}
}

func TestSubscriptionLaunchRejectsUnknownAndDisabled(t *testing.T) {
	s, _ := subsTestServer(t)
	off := false
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {{ID: "main"}, {ID: "gone", Enabled: &off}},
	}
	if _, _, err := s.subscriptionLaunch("claude", "deleted"); err == nil {
		t.Fatal("a removed profile must fail instead of falling back to another account")
	}
	if _, _, err := s.subscriptionLaunch("claude", "gone"); err == nil {
		t.Fatal("a disabled profile must fail")
	}
	if _, _, err := s.subscriptionLaunch("codex", "main"); err == nil {
		t.Fatal("a profile that exists for another provider must not be reused")
	}
	if _, _, err := s.subscriptionLaunch("cursor-agent", "main"); err == nil {
		t.Fatal("a provider without an adapter must fail when a profile is requested")
	}
	if _, _, err := s.subscriptionLaunch("claude", "../escape"); err == nil {
		t.Fatal("a path-traversal id must fail")
	}
}

func TestResolveSubscriptionLabel(t *testing.T) {
	s, _ := subsTestServer(t)
	s.cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main", Name: "Claude Max Main"}}}

	if id, name := s.resolveSubscriptionLabel("claude", ""); id != "" || name != "" {
		t.Fatalf("empty id = (%q, %q), want empty", id, name)
	}
	if id, name := s.resolveSubscriptionLabel("claude", " MAIN "); id != "main" || name != "Claude Max Main" {
		t.Fatalf("normalized lookup = (%q, %q)", id, name)
	}
	// config から消えた profile でも ID は残す（別 profile へ寄せない）。
	if id, name := s.resolveSubscriptionLabel("claude", "removed"); id != "removed" || name != "" {
		t.Fatalf("unknown id = (%q, %q), want (removed, \"\")", id, name)
	}
	if id, _ := s.resolveSubscriptionLabel("claude", "../escape"); id != "" {
		t.Fatalf("invalid id = %q, want empty", id)
	}
}

func TestSubscriptionsAPIListAddUpdateRemove(t *testing.T) {
	s, home := subsTestServer(t)

	// 空の状態でも 200 で、claude セクションが supported として出る。
	w := httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodGet, "/api/subscriptions", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("GET code = %d, want 200: %s", w.Code, w.Body.String())
	}
	var listResp struct {
		Providers []subscription.ProviderEntry `json:"providers"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &listResp); err != nil {
		t.Fatal(err)
	}
	if len(listResp.Providers) == 0 || listResp.Providers[0].Provider != "claude" {
		t.Fatalf("providers = %#v", listResp.Providers)
	}

	// 追加: 表示名から ID を採番する。
	w = httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodPost, "/api/subscriptions", map[string]any{
		"provider": "claude", "name": "Claude Max Main",
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("POST add code = %d: %s", w.Code, w.Body.String())
	}
	if len(s.cfg.Subscriptions["claude"]) != 1 {
		t.Fatalf("profiles = %#v", s.cfg.Subscriptions)
	}
	added := s.cfg.Subscriptions["claude"][0]
	if added.ID != "claude-max-main" {
		t.Fatalf("generated id = %q, want claude-max-main", added.ID)
	}
	dir := filepath.Join(home, ".many-ai-cli", "subscriptions", "claude", added.ID)
	if info, err := os.Stat(dir); err != nil || !info.IsDir() {
		t.Fatalf("profile dir %q not created: %v", dir, err)
	}

	// 同名をもう 1 件足すと ID が衝突しないよう連番になる。
	w = httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodPost, "/api/subscriptions", map[string]any{
		"provider": "claude", "name": "Claude Max Main",
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("POST add #2 code = %d: %s", w.Code, w.Body.String())
	}
	if got := s.cfg.Subscriptions["claude"][1].ID; got != "claude-max-main-2" {
		t.Fatalf("second generated id = %q", got)
	}

	// rename + disable。
	w = httptest.NewRecorder()
	s.handleSubscriptionsItem(w, subsRequest(t, http.MethodPost, "/api/subscriptions/update", map[string]any{
		"provider": "claude", "id": "claude-max-main", "name": "Renamed", "enabled": false,
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("update code = %d: %s", w.Code, w.Body.String())
	}
	updated := s.cfg.Subscriptions["claude"][0]
	if updated.Name != "Renamed" || updated.IsEnabled() {
		t.Fatalf("updated profile = %#v", updated)
	}

	// 無効化した profile は起動に使えない。
	if _, _, err := s.subscriptionLaunch("claude", "claude-max-main"); err == nil {
		t.Fatal("a disabled profile must not launch")
	}

	// 既定の remove は登録解除のみで、認証ディレクトリは残す。
	w = httptest.NewRecorder()
	s.handleSubscriptionsItem(w, subsRequest(t, http.MethodPost, "/api/subscriptions/remove", map[string]any{
		"provider": "claude", "id": "claude-max-main",
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("remove code = %d: %s", w.Code, w.Body.String())
	}
	if len(s.cfg.Subscriptions["claude"]) != 1 {
		t.Fatalf("profiles after remove = %#v", s.cfg.Subscriptions)
	}
	if _, err := os.Stat(dir); err != nil {
		t.Fatalf("profile dir was deleted without delete_credentials: %v", err)
	}

	// delete_credentials を明示したときだけディレクトリを消す。
	w = httptest.NewRecorder()
	s.handleSubscriptionsItem(w, subsRequest(t, http.MethodPost, "/api/subscriptions/remove", map[string]any{
		"provider": "claude", "id": "claude-max-main-2", "delete_credentials": true,
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("remove#2 code = %d: %s", w.Code, w.Body.String())
	}
	dir2 := filepath.Join(home, ".many-ai-cli", "subscriptions", "claude", "claude-max-main-2")
	if _, err := os.Stat(dir2); !os.IsNotExist(err) {
		t.Fatalf("profile dir %q still exists after delete_credentials: %v", dir2, err)
	}
	if len(s.cfg.Subscriptions["claude"]) != 0 {
		t.Fatalf("provider key should be cleaned up: %#v", s.cfg.Subscriptions)
	}
}

func TestSubscriptionsAPIRejectsBadInput(t *testing.T) {
	s, _ := subsTestServer(t)

	cases := []struct {
		name   string
		path   string
		body   map[string]any
		status int
	}{
		{"unsupported provider", "/api/subscriptions", map[string]any{"provider": "cursor-agent", "name": "x"}, http.StatusBadRequest},
		{"missing name", "/api/subscriptions", map[string]any{"provider": "claude", "name": "   "}, http.StatusOK},
		{"traversal id", "/api/subscriptions", map[string]any{"provider": "claude", "id": "../escape", "name": "x"}, http.StatusBadRequest},
	}
	for _, tc := range cases {
		w := httptest.NewRecorder()
		s.handleSubscriptions(w, subsRequest(t, http.MethodPost, tc.path, tc.body))
		if w.Code != tc.status {
			t.Errorf("%s: code = %d, want %d (%s)", tc.name, w.Code, tc.status, w.Body.String())
		}
	}

	// 存在しない profile の update / remove は 404。
	for _, path := range []string{"/api/subscriptions/update", "/api/subscriptions/remove"} {
		w := httptest.NewRecorder()
		s.handleSubscriptionsItem(w, subsRequest(t, http.MethodPost, path, map[string]any{"provider": "claude", "id": "nope"}))
		if w.Code != http.StatusNotFound {
			t.Errorf("%s missing profile: code = %d, want 404", path, w.Code)
		}
	}

	// 未知のアクションは 404。
	w := httptest.NewRecorder()
	s.handleSubscriptionsItem(w, subsRequest(t, http.MethodPost, "/api/subscriptions/bogus", map[string]any{}))
	if w.Code != http.StatusNotFound {
		t.Errorf("unknown action: code = %d, want 404", w.Code)
	}
}

func TestSubscriptionsAPIRequiresToken(t *testing.T) {
	s, _ := subsTestServer(t)
	req := httptest.NewRequest(http.MethodGet, "/api/subscriptions", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:54321"
	w := httptest.NewRecorder()
	s.handleSubscriptions(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("code = %d, want 401", w.Code)
	}
}

// TestSubscriptionsAPIDoesNotReturnCredentials は API 応答に token / 認証ファイル
// 由来の値が出ないことを確認する。応答に出てよいのは ID・表示名・パスまで。
func TestSubscriptionsAPIDoesNotReturnCredentials(t *testing.T) {
	s, home := subsTestServer(t)
	s.cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main", Name: "Main"}}}
	// profile ディレクトリに認証ファイルを置いても、API はその中身を読まない。
	dir := filepath.Join(home, ".many-ai-cli", "subscriptions", "claude", "main")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, ".credentials.json"), []byte(`{"accessToken":"sk-should-never-appear"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	w := httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodGet, "/api/subscriptions", nil))
	body := w.Body.String()
	for _, forbidden := range []string{"sk-should-never-appear", "accessToken", s.cfg.Token} {
		if strings.Contains(body, forbidden) {
			t.Fatalf("response leaked %q:\n%s", forbidden, body)
		}
	}
}

// TestSpawnRejectsUnknownSubscriptionProfile は削除済み profile を指定した spawn が
// **別アカウントで起動せずに** 400 になることを確認する。
func TestSpawnRejectsUnknownSubscriptionProfile(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":                "claude",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": "deleted-profile",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "subscription") {
		t.Fatalf("error body does not name the cause: %s", w.Body.String())
	}
}

func TestSpawnRejectsSubscriptionForShell(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main"}}}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":                "shell",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": "main",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
}

func TestSpawnWrappedSessionRejectsUnknownProfileBeforeStarting(t *testing.T) {
	s, _ := subsTestServer(t)
	if _, err := s.spawnWrappedSession(spawnWrappedSpec{
		Provider:              "claude",
		CWD:                   t.TempDir(),
		Label:                 "orch-test",
		SubscriptionProfileID: "deleted-profile",
	}, 0); err == nil {
		t.Fatal("an orchestration child must not start with an unknown subscription profile")
	}
}

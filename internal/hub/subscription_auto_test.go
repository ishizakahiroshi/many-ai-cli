package hub

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

func autoTestServer(t *testing.T) *Server {
	t.Helper()
	s, _ := subsTestServer(t)
	off := false
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {
			{ID: "a", Name: "A"},
			{ID: "b", Name: "B"},
			{ID: "off", Name: "Off", Enabled: &off},
			{ID: "../broken"},
		},
	}
	return s
}

// TestAutoSubscriptionRoundRobin は auto が有効 profile を順番に使い、
// 同じ profile へ偏らないことを確認する。
func TestAutoSubscriptionRoundRobin(t *testing.T) {
	s := autoTestServer(t)
	seen := make([]string, 0, 4)
	for i := 0; i < 4; i++ {
		_, resolved, err := s.subscriptionLaunch("claude", "auto")
		if err != nil {
			t.Fatalf("auto launch #%d: %v", i, err)
		}
		seen = append(seen, resolved.ID)
	}
	if strings.Join(seen, ",") != "a,b,a,b" {
		t.Fatalf("auto picks = %v, want a,b,a,b", seen)
	}
}

// TestAutoSubscriptionRecordsConcreteProfile は auto で起動したセッションに
// 「auto」ではなく実際に選ばれた profile ID が残ることを確認する。
func TestAutoSubscriptionRecordsConcreteProfile(t *testing.T) {
	s := autoTestServer(t)
	env, resolved, err := s.subscriptionLaunch("claude", "auto")
	if err != nil {
		t.Fatal(err)
	}
	if resolved.ID == "auto" || resolved.ID == "" {
		t.Fatalf("resolved id = %q, want a concrete profile id", resolved.ID)
	}
	joined := strings.Join(env, "\n")
	if !strings.Contains(joined, subscription.SessionEnvVar+"="+resolved.ID) {
		t.Fatalf("env %v does not carry the concrete profile id", env)
	}
	if strings.Contains(joined, subscription.SessionEnvVar+"=auto") {
		t.Fatalf("env %v records the keyword instead of the chosen profile", env)
	}
}

// TestAutoSubscriptionSkipsDisabledAndBroken は無効化・設定不正の profile が
// auto の候補に入らないことを確認する。
func TestAutoSubscriptionSkipsDisabledAndBroken(t *testing.T) {
	s := autoTestServer(t)
	for i := 0; i < 6; i++ {
		_, resolved, err := s.subscriptionLaunch("claude", "auto")
		if err != nil {
			t.Fatal(err)
		}
		if resolved.ID == "off" || strings.Contains(resolved.ID, "..") {
			t.Fatalf("auto picked an ineligible profile: %q", resolved.ID)
		}
	}
}

// TestAutoSubscriptionWithoutCandidatesFails は候補 0 件のときに既定ログインへ
// 黙って落ちないことを確認する。auto を選んだ利用者は「登録した契約のどれかで
// 動く」ことを期待しているので、0 件は設定の問題として見せる。
func TestAutoSubscriptionWithoutCandidatesFails(t *testing.T) {
	s, _ := subsTestServer(t)
	if _, _, err := s.subscriptionLaunch("claude", "auto"); err == nil {
		t.Fatal("auto with no registered profile must fail")
	}
	off := false
	s.cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "only", Enabled: &off}}}
	if _, _, err := s.subscriptionLaunch("claude", "auto"); err == nil {
		t.Fatal("auto with only disabled profiles must fail")
	}
}

// TestAutoIsReservedAndCannotBeRegistered は "auto" という名前の profile を
// 作れないことを確認する。作れてしまうと、その profile を明示指定できなくなる。
func TestAutoIsReservedAndCannotBeRegistered(t *testing.T) {
	s, _ := subsTestServer(t)
	w := httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodPost, "/api/subscriptions", map[string]any{
		"provider": "claude", "id": "auto", "name": "Auto",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if len(s.cfg.Subscriptions["claude"]) != 0 {
		t.Fatalf("a reserved id was registered: %#v", s.cfg.Subscriptions)
	}
}

// TestLiveSessionAuthIsNeverSwapped は「実行中セッションの認証を途中で差し替えない」
// という親 plan の禁止事項を、コード上の不変条件として固定する。
//
// 振る舞いテストでは「差し替える経路が無いこと」を示せない（無いものは呼べない）。
// そこで代入箇所そのものを検査し、register / reattach 以外から書かれていないことを見る。
func TestLiveSessionAuthIsNeverSwapped(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	const field = "SubscriptionProfileID"
	allowed := map[string]bool{"wrapper_loop.go": true}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || filepath.Ext(name) != ".go" || strings.HasSuffix(name, "_test.go") {
			continue
		}
		body, err := os.ReadFile(name)
		if err != nil {
			t.Fatal(err)
		}
		for _, line := range strings.Split(string(body), "\n") {
			trimmed := strings.TrimSpace(line)
			// 構造体リテラルのフィールド指定（`Field: value,`）は生成時なので対象外。
			if !strings.Contains(trimmed, field+" =") && !strings.Contains(trimmed, field+"=") {
				continue
			}
			if allowed[name] {
				continue
			}
			t.Errorf("%s assigns %s outside session creation: %s", name, field, trimmed)
		}
	}
}

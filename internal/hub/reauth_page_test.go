package hub

import (
	"net/http"
	"net/http/httptest"
	"regexp"
	"strings"
	"testing"
)

// 2026-08-17: ホーム画面へ追加した PWA の cold launch が毎回 401 になり、
// 401 の本文が JSON だったためフロントが 1 行も動かず復帰できなかった件の回帰テスト。
// 直したのは 2 点で、片方だけ戻ると症状が再発するので両方をここで固定する。
//   1. token cookie の SameSite を Strict → Lax（ブラウザ外から始まる遷移に cookie を乗せる）
//   2. 未認証のページ遷移には JSON ではなく再認証ページを返す（復帰経路を作る）

func reauthTestRequest(accept string) *http.Request {
	req := httptest.NewRequest(http.MethodGet, "/", nil)
	req.Host = "127.0.0.1:47777"
	if accept != "" {
		req.Header.Set("Accept", accept)
	}
	return req
}

// TestTokenCookieUsesLaxSameSite は認証 cookie が Lax であることを固定する。
// Strict へ戻すと、ホーム画面起動のトップレベル遷移に cookie が乗らなくなる。
func TestTokenCookieUsesLaxSameSite(t *testing.T) {
	s := auditFixServer(t, false)
	req := httptest.NewRequest(http.MethodGet, "/?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleIndex(w, req)

	var found *http.Cookie
	for _, c := range w.Result().Cookies() {
		if c.Name == tokenCookieName {
			found = c
			break
		}
	}
	if found == nil {
		t.Fatal("expected Set-Cookie for valid token request")
	}
	if found.SameSite != http.SameSiteLaxMode {
		t.Fatalf("cookie SameSite = %v, want %v (Lax)", found.SameSite, http.SameSiteLaxMode)
	}
}

// TestHandleIndexServesReauthPageForUnauthenticatedNavigation は未認証のページ遷移へ
// HTML の再認証ページが返ることを確認する。ステータスは 401 のまま変えない。
func TestHandleIndexServesReauthPageForUnauthenticatedNavigation(t *testing.T) {
	s := auditFixServer(t, false)
	w := httptest.NewRecorder()
	s.handleIndex(w, reauthTestRequest("text/html,application/xhtml+xml"))

	res := w.Result()
	if res.StatusCode != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", res.StatusCode, http.StatusUnauthorized)
	}
	if ct := res.Header.Get("Content-Type"); !strings.HasPrefix(ct, "text/html") {
		t.Fatalf("Content-Type = %q, want text/html", ct)
	}
	body := w.Body.String()
	// 復帰に必要な 2 つのキーが実際に埋まっていること（プレースホルダのまま出荷しない）。
	if !strings.Contains(body, reauthTokenStorageKey) {
		t.Errorf("body does not contain token storage key %q", reauthTokenStorageKey)
	}
	if !strings.Contains(body, reauthGuardKey) {
		t.Errorf("body does not contain retry guard key %q", reauthGuardKey)
	}
	if strings.Contains(body, "__NONCE__") || strings.Contains(body, "__TOKEN_KEY__") || strings.Contains(body, "__GUARD_KEY__") {
		t.Error("body still contains an unreplaced placeholder")
	}
	// 未認証なので token を配ってはいけない。
	for _, c := range res.Cookies() {
		if c.Name == tokenCookieName {
			t.Fatal("reauth page must not issue the token cookie")
		}
	}
}

// TestReauthPageNonceMatchesCSP は inline script が自分の CSP で実際に実行できる
// こと（ヘッダの nonce と本文の nonce が同一）を確認する。ズレると画面が無反応になる。
func TestReauthPageNonceMatchesCSP(t *testing.T) {
	s := auditFixServer(t, false)
	w := httptest.NewRecorder()
	s.handleIndex(w, reauthTestRequest("text/html"))

	csp := w.Result().Header.Get("Content-Security-Policy")
	m := regexp.MustCompile(`script-src 'nonce-([0-9a-f]+)'`).FindStringSubmatch(csp)
	if m == nil {
		t.Fatalf("CSP has no script-src nonce: %q", csp)
	}
	nonce := m[1]
	if !strings.Contains(w.Body.String(), `<script nonce="`+nonce+`">`) {
		t.Fatal("script tag nonce does not match the CSP nonce")
	}
	// allowed_hosts を未認証の相手へ晒さないため、documentCSP は使わない。
	if strings.Contains(csp, "ws://") || strings.Contains(csp, "wss://") {
		t.Errorf("reauth CSP must not expand allowed_hosts: %q", csp)
	}
}

// TestHandleIndexKeepsJSONForUnauthenticatedNonDocument は curl / fetch など
// ページ遷移ではない要求の 401 JSON が従来どおりであることを確認する。
func TestHandleIndexKeepsJSONForUnauthenticatedNonDocument(t *testing.T) {
	for _, accept := range []string{"", "application/json", "image/png"} {
		w := httptest.NewRecorder()
		auditFixServer(t, false).handleIndex(w, reauthTestRequest(accept))

		res := w.Result()
		if res.StatusCode != http.StatusUnauthorized {
			t.Errorf("accept=%q: status = %d, want 401", accept, res.StatusCode)
		}
		if ct := res.Header.Get("Content-Type"); !strings.HasPrefix(ct, "application/json") {
			t.Errorf("accept=%q: Content-Type = %q, want application/json", accept, ct)
		}
		if body := strings.TrimSpace(w.Body.String()); body != `{"ok":false,"error":"unauthorized","detail":"unauthorized"}` {
			t.Errorf("accept=%q: body = %q", accept, body)
		}
	}
}

// TestHandleIndexSecFetchDestDocumentGetsReauthPage は Accept を持たなくても
// Sec-Fetch-Dest: document ならページ遷移として扱うことを確認する。
func TestHandleIndexSecFetchDestDocumentGetsReauthPage(t *testing.T) {
	s := auditFixServer(t, false)
	req := reauthTestRequest("")
	req.Header.Set("Sec-Fetch-Dest", "document")
	w := httptest.NewRecorder()
	s.handleIndex(w, req)

	if ct := w.Result().Header.Get("Content-Type"); !strings.HasPrefix(ct, "text/html") {
		t.Fatalf("Content-Type = %q, want text/html", ct)
	}
}

// TestReauthPageNotServedWhenHostNotAllowed は DNS リバインディング防御が
// 再認証ページより先に効くことを確認する。未認証でも Host 検証は素通しさせない。
func TestReauthPageNotServedWhenHostNotAllowed(t *testing.T) {
	s := auditFixServer(t, false)
	req := reauthTestRequest("text/html")
	req.Host = "evil.example"
	w := httptest.NewRecorder()
	s.handleIndex(w, req)

	if w.Result().StatusCode != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", w.Result().StatusCode, http.StatusForbidden)
	}
	if strings.Contains(w.Body.String(), reauthTokenStorageKey) {
		t.Fatal("reauth page must not be served to a disallowed Host")
	}
}

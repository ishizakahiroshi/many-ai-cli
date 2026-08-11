package hub

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

const (
	testLoopbackAddr = "127.0.0.1:5000"
	testRemoteAddr   = "203.0.113.9:40000"
	testHubHost      = "127.0.0.1:47777"
)

func newPINTestServer(t *testing.T, pin string) *Server {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"
	s.cfg.AuthCookieSecret = "test-secret-0123456789abcdef"
	hash, err := hashPIN(pin)
	if err != nil {
		t.Fatalf("hashPIN: %v", err)
	}
	s.cfg.RemotePINHash = hash
	return s
}

func TestIsValidPINFormat(t *testing.T) {
	cases := map[string]bool{
		"123456":   true,
		"12345":    false, // 5 桁は不可
		"000000":   true,
		"12345678": true,
		"12345a":   false, // 数字以外
		"":         false,
		"12 3456":  false,
	}
	for in, want := range cases {
		if got := isValidPINFormat(in); got != want {
			t.Errorf("isValidPINFormat(%q) = %v, want %v", in, got, want)
		}
	}
}

func TestPINCookieRoundTrip(t *testing.T) {
	secret := "abc-secret"
	now := time.Now()
	val := signPINCookie(secret, now.Add(time.Hour).Unix())
	if !verifyPINCookie(secret, val, now) {
		t.Fatal("valid cookie should verify")
	}
	if verifyPINCookie("other-secret", val, now) {
		t.Fatal("wrong secret must not verify (revoke-all rotation invalidates cookies)")
	}
	expired := signPINCookie(secret, now.Add(-time.Second).Unix())
	if verifyPINCookie(secret, expired, now) {
		t.Fatal("expired cookie must not verify")
	}
	if verifyPINCookie(secret, "garbage", now) {
		t.Fatal("malformed cookie must not verify")
	}
}

func TestPINCookieCarriesNonceAndDeviceHash(t *testing.T) {
	secret := "abc-secret"
	now := time.Now()
	expiry := now.Add(time.Hour).Unix()
	val := signPINCookie(secret, expiry, "nonce-1", "device-1")
	gotExpiry, nonce, deviceHash, ok := parsePINCookie(secret, val)
	if !ok || gotExpiry != expiry || nonce != "nonce-1" || deviceHash != "device-1" {
		t.Fatalf("parsePINCookie = (%d,%q,%q,%v)", gotExpiry, nonce, deviceHash, ok)
	}
	if !verifyPINCookie(secret, val, now, "device-1") {
		t.Fatal("cookie should verify for its issuing device")
	}
	if verifyPINCookie(secret, val, now, "device-2") {
		t.Fatal("cookie must not verify for another device")
	}
}

func TestPINRateLimitKeyForLoopbackProxy(t *testing.T) {
	s := newTestServer()
	req := httptest.NewRequest(http.MethodPost, "/api/auth/login", nil)
	req.RemoteAddr = testLoopbackAddr
	req.Host = "host.example.ts.net"
	if got := s.pinRateLimitKey(req); got != "" {
		t.Fatalf("anonymous proxy key = %q, want global-only empty key", got)
	}
	req.Header.Set("Tailscale-User-Login", "alice@example.com")
	alice := s.pinRateLimitKey(req)
	req.Header.Set("Tailscale-User-Login", "bob@example.com")
	bob := s.pinRateLimitKey(req)
	if alice == "" || bob == "" || alice == bob {
		t.Fatalf("identity keys must be non-empty and distinct: alice=%q bob=%q", alice, bob)
	}
}

func TestRequireRemotePIN_LoopbackBypass(t *testing.T) {
	s := newPINTestServer(t, "123456")
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testLoopbackAddr
	req.Host = testHubHost // 実ローカルアクセス（既定ホスト）。tailscale serve 等の非既定ホストとは区別する
	w := httptest.NewRecorder()
	if !s.requireRemotePIN(w, req) {
		t.Fatalf("loopback should bypass PIN, got %d", w.Code)
	}
}

// TestRequireRemotePIN_ProxiedRemoteRequiresCookie は tailscale serve 等の
// リバースプロキシ経由（TCP 元は loopback だが Host が tailnet DNS 名＝非既定の
// allowed_host）では PIN が要求されることを検証する（loopback 偽装での PIN 素通し防止）。
func TestRequireRemotePIN_ProxiedRemoteRequiresCookie(t *testing.T) {
	s := newPINTestServer(t, "123456")
	s.cfg.Hub.AllowedHosts = []string{"host.example.ts.net"}
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testLoopbackAddr // serve プロキシは 127.0.0.1 から再接続する
	req.Host = "host.example.ts.net"
	w := httptest.NewRecorder()
	if s.requireRemotePIN(w, req) {
		t.Fatal("tailscale serve 経由（非既定ホスト）は PIN cookie 無しでは通してはいけない")
	}
	if w.Code != http.StatusUnauthorized {
		t.Errorf("code = %d, want 401", w.Code)
	}
}

func TestRequireRemotePIN_RemoteRequiresCookie(t *testing.T) {
	s := newPINTestServer(t, "123456")
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testRemoteAddr
	w := httptest.NewRecorder()
	if s.requireRemotePIN(w, req) {
		t.Fatal("remote without PIN cookie should be rejected")
	}
	if w.Code != http.StatusUnauthorized {
		t.Errorf("code = %d, want 401", w.Code)
	}
}

func TestRequireRemotePIN_DisabledWhenNoHash(t *testing.T) {
	s := newTestServer()
	s.cfg.RemotePINHash = "" // PIN 無効
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testRemoteAddr
	w := httptest.NewRecorder()
	if !s.requireRemotePIN(w, req) {
		t.Fatal("disabled PIN should let remote through")
	}
}

func loginReq(pin string) *http.Request {
	body, _ := json.Marshal(map[string]string{"pin": pin})
	req := httptest.NewRequest(http.MethodPost, "/api/auth/login?token=tok", strings.NewReader(string(body)))
	req.Host = testHubHost
	req.RemoteAddr = testRemoteAddr
	return req
}

func TestAuthLogin_WrongThenCorrect(t *testing.T) {
	s := newPINTestServer(t, "123456")

	// 誤 PIN → 401 bad_pin
	w := httptest.NewRecorder()
	s.handleAuthLogin(w, loginReq("000000"))
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("wrong pin code = %d, want 401", w.Code)
	}

	// 正 PIN → 200 + Set-Cookie
	w2 := httptest.NewRecorder()
	s.handleAuthLogin(w2, loginReq("123456"))
	if w2.Code != http.StatusOK {
		t.Fatalf("correct pin code = %d, want 200", w2.Code)
	}
	var cookie *http.Cookie
	for _, c := range w2.Result().Cookies() {
		if c.Name == pinCookieName {
			cookie = c
		}
	}
	if cookie == nil || cookie.Value == "" {
		t.Fatal("expected PIN session cookie to be set on success")
	}

	// その cookie を提示すれば remote ゲートを通過する。
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testRemoteAddr
	req.AddCookie(cookie)
	w3 := httptest.NewRecorder()
	if !s.requireRemotePIN(w3, req) {
		t.Fatalf("valid PIN cookie should pass gate, got %d", w3.Code)
	}
}

func TestAuthLoginSecureCookieThroughHTTPSProxy(t *testing.T) {
	s := newPINTestServer(t, "123456")
	s.cfg.Hub.AllowedHosts = []string{"host.example.ts.net"}
	req := loginReq("123456")
	req.RemoteAddr = testLoopbackAddr
	req.Host = "host.example.ts.net"
	req.Header.Set("X-Forwarded-Proto", "https")
	w := httptest.NewRecorder()
	s.handleAuthLogin(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("login code = %d, want 200", w.Code)
	}
	for _, cookie := range w.Result().Cookies() {
		if cookie.Name == pinCookieName && !cookie.Secure {
			t.Fatal("PIN cookie over an HTTPS proxy must be Secure")
		}
	}
}

func TestAuthLogoutRevokesOnlyPresentedPINSession(t *testing.T) {
	s := newPINTestServer(t, "123456")
	login := func() *http.Cookie {
		w := httptest.NewRecorder()
		s.handleAuthLogin(w, loginReq("123456"))
		for _, c := range w.Result().Cookies() {
			if c.Name == pinCookieName {
				return c
			}
		}
		t.Fatal("PIN cookie missing")
		return nil
	}
	first := login()
	second := login()
	req := httptest.NewRequest(http.MethodPost, "/api/auth/logout?token=tok", nil)
	req.Host = testHubHost
	req.RemoteAddr = testRemoteAddr
	req.AddCookie(first)
	w := httptest.NewRecorder()
	s.handleAuthLogout(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("logout code = %d", w.Code)
	}
	check := func(cookie *http.Cookie) bool {
		r := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
		r.RemoteAddr = testRemoteAddr
		r.AddCookie(cookie)
		return s.hasValidPINCookie(r)
	}
	if check(first) {
		t.Fatal("presented PIN session remained valid after logout")
	}
	if !check(second) {
		t.Fatal("logout revoked an unrelated PIN session")
	}
}

func TestIssuePINCookieEvictsOldestSessionAtCapacity(t *testing.T) {
	s := newTestServer()
	now := time.Now()
	expiry := now.Add(pinCookieTTL).Unix()
	s.pinSessions = make(map[string]pinCookieSession, maxPINSessions)
	for i := 0; i < maxPINSessions; i++ {
		nonce := fmt.Sprintf("nonce-%03d", i)
		s.pinSessions[nonce] = pinCookieSession{
			expiry:     expiry,
			deviceHash: "device",
			issuedAt:   int64(i + 1),
		}
	}
	req := httptest.NewRequest(http.MethodPost, "https://example.test/api/auth/login", nil)
	req.RemoteAddr = testRemoteAddr
	value, err := s.issuePINCookie("secret", req, expiry)
	if err != nil {
		t.Fatalf("issuePINCookie: %v", err)
	}
	if got := len(s.pinSessions); got != maxPINSessions {
		t.Fatalf("pin session count = %d, want %d", got, maxPINSessions)
	}
	if _, exists := s.pinSessions["nonce-000"]; exists {
		t.Fatal("oldest PIN session was not evicted")
	}
	_, nonce, _, ok := parsePINCookie("secret", value)
	if !ok || nonce == "" {
		t.Fatalf("issued cookie is invalid: ok=%v nonce=%q", ok, nonce)
	}
	if _, exists := s.pinSessions[nonce]; !exists {
		t.Fatal("new PIN session was not retained")
	}
}

func TestIssuePINCookiePrunesExpiredBeforeEviction(t *testing.T) {
	s := newTestServer()
	now := time.Now()
	s.pinSessions = map[string]pinCookieSession{
		"expired": {
			expiry:     now.Add(-time.Minute).Unix(),
			deviceHash: "device",
			issuedAt:   1,
		},
		"active": {
			expiry:     now.Add(time.Hour).Unix(),
			deviceHash: "device",
			issuedAt:   2,
		},
	}
	req := httptest.NewRequest(http.MethodPost, "https://example.test/api/auth/login", nil)
	req.RemoteAddr = testRemoteAddr
	if _, err := s.issuePINCookie("secret", req, now.Add(pinCookieTTL).Unix()); err != nil {
		t.Fatalf("issuePINCookie: %v", err)
	}
	if _, exists := s.pinSessions["expired"]; exists {
		t.Fatal("expired PIN session was not pruned")
	}
	if _, exists := s.pinSessions["active"]; !exists {
		t.Fatal("active PIN session was evicted while capacity was available")
	}
}

func TestDeviceKeyUsesCoarseUserAgentBucket(t *testing.T) {
	a := deviceKey(testRemoteAddr, "Mozilla/5.0 (Windows NT 10.0) Chrome/120.0")
	b := deviceKey(testRemoteAddr, "Mozilla/5.0 (Windows NT 10.0) Chrome/999.0")
	if a != b {
		t.Fatalf("Chrome version churn changed device bucket: %q vs %q", a, b)
	}
	c := deviceKey(testRemoteAddr, "Mozilla/5.0 (Windows NT 10.0) Firefox/120.0")
	if a == c {
		t.Fatal("different browser brands collapsed into one device bucket")
	}
}

func TestAuthLogin_Lockout(t *testing.T) {
	s := newPINTestServer(t, "123456")
	// pinLockThreshold-1 回は 401、しきい値到達でロック → 429。
	for i := 0; i < pinLockThreshold-1; i++ {
		w := httptest.NewRecorder()
		s.handleAuthLogin(w, loginReq("000000"))
		if w.Code != http.StatusUnauthorized {
			t.Fatalf("attempt %d code = %d, want 401", i+1, w.Code)
		}
	}
	w := httptest.NewRecorder()
	s.handleAuthLogin(w, loginReq("000000"))
	if w.Code != http.StatusTooManyRequests {
		t.Fatalf("threshold attempt code = %d, want 429", w.Code)
	}
	// ロック中は正しい PIN でも 429。
	w2 := httptest.NewRecorder()
	s.handleAuthLogin(w2, loginReq("123456"))
	if w2.Code != http.StatusTooManyRequests {
		t.Fatalf("locked-out correct pin code = %d, want 429", w2.Code)
	}
}

func TestAuthStatus_RemoteUnauthed(t *testing.T) {
	s := newPINTestServer(t, "123456")
	req := httptest.NewRequest(http.MethodGet, "/api/auth/status?token=tok", nil)
	req.Host = testHubHost
	req.RemoteAddr = testRemoteAddr
	w := httptest.NewRecorder()
	s.handleAuthStatus(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}
	var resp map[string]any
	if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
		t.Fatalf("decode: %v", err)
	}
	if resp["pin_enabled"] != true {
		t.Errorf("pin_enabled = %v, want true", resp["pin_enabled"])
	}
	if resp["authed"] != false {
		t.Errorf("authed = %v, want false", resp["authed"])
	}
}

func TestAuthStatus_LoopbackAuthed(t *testing.T) {
	s := newPINTestServer(t, "123456")
	req := httptest.NewRequest(http.MethodGet, "/api/auth/status?token=tok", nil)
	req.Host = testHubHost
	req.RemoteAddr = testLoopbackAddr
	w := httptest.NewRecorder()
	s.handleAuthStatus(w, req)
	var resp map[string]any
	_ = json.NewDecoder(w.Body).Decode(&resp)
	if resp["authed"] != true {
		t.Errorf("loopback authed = %v, want true", resp["authed"])
	}
}

func TestAuthSetPIN_SetAndClear(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Token = "tok"

	// loopback から PIN 設定（初回）。
	setBody, _ := json.Marshal(map[string]any{"pin": "135790"})
	req := httptest.NewRequest(http.MethodPost, "/api/auth/set-pin?token=tok", strings.NewReader(string(setBody)))
	req.Host = testHubHost
	req.RemoteAddr = testLoopbackAddr
	w := httptest.NewRecorder()
	s.handleAuthSetPIN(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("set pin code = %d, want 200", w.Code)
	}
	if s.cfg.RemotePINHash == "" {
		t.Fatal("RemotePINHash should be set")
	}
	if s.cfg.AuthCookieSecret == "" {
		t.Fatal("AuthCookieSecret should be auto-generated when enabling PIN")
	}
	if !verifyPIN(s.cfg.RemotePINHash, "135790") {
		t.Fatal("stored hash should verify the set PIN")
	}

	// 不正フォーマットは 400。
	badBody, _ := json.Marshal(map[string]any{"pin": "12ab"})
	reqBad := httptest.NewRequest(http.MethodPost, "/api/auth/set-pin?token=tok", strings.NewReader(string(badBody)))
	reqBad.Host = testHubHost
	reqBad.RemoteAddr = testLoopbackAddr
	wBad := httptest.NewRecorder()
	s.handleAuthSetPIN(wBad, reqBad)
	if wBad.Code != http.StatusBadRequest {
		t.Fatalf("bad pin format code = %d, want 400", wBad.Code)
	}

	// 解除。
	clearBody, _ := json.Marshal(map[string]any{"clear": true})
	reqClr := httptest.NewRequest(http.MethodPost, "/api/auth/set-pin?token=tok", strings.NewReader(string(clearBody)))
	reqClr.Host = testHubHost
	reqClr.RemoteAddr = testLoopbackAddr
	wClr := httptest.NewRecorder()
	s.handleAuthSetPIN(wClr, reqClr)
	if wClr.Code != http.StatusOK {
		t.Fatalf("clear pin code = %d, want 200", wClr.Code)
	}
	if s.cfg.RemotePINHash != "" {
		t.Fatal("RemotePINHash should be cleared")
	}
}

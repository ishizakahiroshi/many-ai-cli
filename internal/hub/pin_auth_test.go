package hub

import (
	"encoding/json"
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

func TestRequireRemotePIN_LoopbackBypass(t *testing.T) {
	s := newPINTestServer(t, "123456")
	req := httptest.NewRequest(http.MethodGet, "/api/anything", nil)
	req.RemoteAddr = testLoopbackAddr
	w := httptest.NewRecorder()
	if !s.requireRemotePIN(w, req) {
		t.Fatalf("loopback should bypass PIN, got %d", w.Code)
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

package hub

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// uiOriginCookieName is a Hub-minted capability distinct from MANY_AI_CLI_token.
// Fetch Metadata alone is forgeable by any Hub-token holder (wrappers inject
// MANY_AI_CLI_HUB_TOKEN). This cookie is signed with AuthCookieSecret or a
// process-local secret that is never placed in wrapper environments, so a
// spawn-child that only forges Sec-Fetch-* cannot claim origin:"ui" (F-AI-03).
const uiOriginCookieName = "MANY_AI_CLI_ui_origin"

const uiOriginCookieTTL = 24 * time.Hour

const uiOriginCookiePurpose = "ui-origin-v1"

func (s *Server) uiOriginHMACSecret() string {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	if s.cfg != nil && s.cfg.AuthCookieSecret != "" {
		return s.cfg.AuthCookieSecret
	}
	if s.uiOriginSecret == "" {
		b := make([]byte, 32)
		if _, err := rand.Read(b); err != nil {
			// Extremely unlikely; fall back to a non-empty deterministic marker
			// so verify fails closed rather than panicking.
			s.uiOriginSecret = "ui-origin-fallback"
		} else {
			s.uiOriginSecret = hex.EncodeToString(b)
		}
	}
	return s.uiOriginSecret
}

func signUIOriginCookie(secret string, expiry int64, nonce string) string {
	payload := strconv.FormatInt(expiry, 10) + "." + nonce
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(uiOriginCookiePurpose + ":" + payload))
	return payload + "." + hex.EncodeToString(mac.Sum(nil))
}

func verifyUIOriginCookie(secret, value string, now time.Time) bool {
	if secret == "" || value == "" {
		return false
	}
	dot := strings.LastIndex(value, ".")
	if dot <= 0 || dot == len(value)-1 {
		return false
	}
	payload, sig := value[:dot], value[dot+1:]
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(uiOriginCookiePurpose + ":" + payload))
	want := hex.EncodeToString(mac.Sum(nil))
	if subtle.ConstantTimeCompare([]byte(sig), []byte(want)) != 1 {
		return false
	}
	parts := strings.Split(payload, ".")
	if len(parts) != 2 || parts[1] == "" {
		return false
	}
	expiry, err := strconv.ParseInt(parts[0], 10, 64)
	if err != nil || now.Unix() >= expiry {
		return false
	}
	return true
}

func (s *Server) issueUIOriginCookie(w http.ResponseWriter, r *http.Request) {
	if w == nil {
		return
	}
	secret := s.uiOriginHMACSecret()
	nonceBytes := make([]byte, 16)
	if _, err := rand.Read(nonceBytes); err != nil {
		return
	}
	expiry := time.Now().Add(uiOriginCookieTTL).Unix()
	value := signUIOriginCookie(secret, expiry, hex.EncodeToString(nonceBytes))
	http.SetCookie(w, &http.Cookie{
		Name:     uiOriginCookieName,
		Value:    value,
		Path:     "/",
		HttpOnly: true,
		Secure:   requestUsesHTTPS(r),
		SameSite: http.SameSiteLaxMode,
		MaxAge:   int(uiOriginCookieTTL / time.Second),
	})
}

func (s *Server) hasValidUIOriginCookie(r *http.Request) bool {
	if r == nil {
		return false
	}
	c, err := r.Cookie(uiOriginCookieName)
	if err != nil || c == nil {
		return false
	}
	return verifyUIOriginCookie(s.uiOriginHMACSecret(), strings.TrimSpace(c.Value), time.Now())
}

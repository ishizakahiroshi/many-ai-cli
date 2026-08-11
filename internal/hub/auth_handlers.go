package hub

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net/http"
	neturl "net/url"
)

// randomHex は n バイトの暗号乱数を 16 進文字列で返す。
func randomHex(n int) (string, error) {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		return "", fmt.Errorf("generate random: %w", err)
	}
	return hex.EncodeToString(b), nil
}

// handleAuthLogout は MANY_AI_CLI_token / MANY_AI_CLI_pin Cookie を MaxAge=-1 で
// 明示失効させる。C5 (plan_audit_score_s_promotion_2026-07-05.md) のロールバック用:
// 端末を貸した後などに「発行済み HttpOnly Cookie を今すぐ切りたい」需要に応える。
// token は revoke-all で消えるが、logout は revoke なしで手元の Cookie だけ捨てる軽い経路。
//
// 認証は guardBase（token + method + Host + Origin）で PIN ゲート抜きに課す:
// - Cookie を持つ = 認証済み → その Cookie で認証が通り自分の Cookie を失効させられる
// - Cookie 無し = そもそも logout 対象がない → 401 で拒否しても意味は変わらない
// これにより CSRF / 他人による強制ログアウト経路を閉じる（TestRegisteredAPIRoutesRequireToken の期待も満たす）。
func (s *Server) handleAuthLogout(w http.ResponseWriter, r *http.Request) {
	if !s.guardBase(w, r, http.MethodPost) {
		return
	}
	s.cfgMu.Lock()
	secret := s.cfg.AuthCookieSecret
	s.cfgMu.Unlock()
	if c, err := r.Cookie(pinCookieName); err == nil {
		s.revokePINCookie(c.Value, secret)
	}
	http.SetCookie(w, &http.Cookie{
		Name:     tokenCookieName,
		Value:    "",
		Path:     "/",
		HttpOnly: true,
		Secure:   requestUsesHTTPS(r),
		SameSite: http.SameSiteStrictMode,
		MaxAge:   -1, // -1 → Max-Age=0 を送出しブラウザは即削除
	})
	http.SetCookie(w, &http.Cookie{
		Name:     pinCookieName,
		Value:    "",
		Path:     "/",
		HttpOnly: true,
		Secure:   requestUsesHTTPS(r),
		SameSite: http.SameSiteStrictMode,
		MaxAge:   -1,
	})
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{"ok": true})
}

// handleAuthRevokeAll は「全アクセス失効」キルスイッチ
// （plan_hub-remote-auth.md / B）。cfg.Token と AuthCookieSecret を再生成して
// 永続化する。これにより既存の token URL・token cookie・（将来の）認証 cookie が
// すべて無効化され、紛失端末を含む全デバイスが Hub から弾かれる。
// レスポンスで新 token / 新 URL を返し、UI は新 URL へ誘導する（このPC自身も切れる）。
//
// 注意: SSH/VPN 経路の鍵失効は別途必要（manual_mobile-access.md の紛失時プレイブック）。
// Hub token を消しても SSH 経路は塞がらない。
func (s *Server) handleAuthRevokeAll(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	// 追加ゲート: PIN 未設定時の bootstrap で remotePINRequired() が false を返すため、
	// guard() 単体ではリモート token 保持者が token + AuthCookieSecret を rotate して
	// 所有者を締め出せる。loopback でないリモートは既存 PIN cookie で本人確認できる
	// 場合のみ通す。
	if s.isLogicallyRemote(r) && !s.hasValidPINCookie(r) {
		w.Header().Set("Cache-Control", "no-store")
		writeJSONError(w, http.StatusForbidden, "forbidden", "revoke-all from a remote address requires existing PIN authentication or a local (loopback) session")
		return
	}
	newToken, err := randomHex(32)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to generate token")
		return
	}
	newSecret, err := randomHex(32)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to generate secret")
		return
	}
	s.cfgMu.Lock()
	prevToken := s.cfg.Token
	prevSecret := s.cfg.AuthCookieSecret
	s.cfg.Token = newToken
	s.cfg.AuthCookieSecret = newSecret
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		// persist に失敗するとディスクは旧値のまま、in-memory は新値になり、
		// 新 token はレスポンスにも載らないので Hub への外部アクセスが完全に
		// 塞がる（プロセス再起動が必要）。in-memory を巻き戻して失敗を返す。
		s.cfgMu.Lock()
		s.cfg.Token = prevToken
		s.cfg.AuthCookieSecret = prevSecret
		s.cfgMu.Unlock()
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to persist config")
		return
	}
	s.pinSessionsMu.Lock()
	s.pinSessions = map[string]pinCookieSession{}
	s.pinSessionsMu.Unlock()
	port := s.currentHubPort()
	// 新 token を含むためキャッシュ禁止。
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"token":   newToken,
		"hub_url": fmt.Sprintf("http://127.0.0.1:%d/?token=%s", port, neturl.QueryEscape(newToken)),
	})
}

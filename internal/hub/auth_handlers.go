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
	s.cfg.Token = newToken
	s.cfg.AuthCookieSecret = newSecret
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal", "failed to persist config")
		return
	}
	port := s.currentHubPort()
	// 新 token を含むためキャッシュ禁止。
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"token":   newToken,
		"hub_url": fmt.Sprintf("http://127.0.0.1:%d/?token=%s", port, neturl.QueryEscape(newToken)),
	})
}

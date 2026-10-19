package hub

import (
	"fmt"
	"net/http"
	neturl "net/url"
)

// Tailscale モバイル接続エンドポイント（plan_mobile-connect-flow-redesign.md C1）。
//
// すべて guard 必須（token＋host＋Origin の三重）。token は CLI へ渡さず、
// ログにも出さない（既存 G7 踏襲）。bind は 127.0.0.1 不変・Funnel は使わない。

// tailscaleHTTPSURL は ready 時の本物 URL（https://<DNSName>/?token=<token>）を組み立てる。
// DNSName が空なら空文字を返す。
func tailscaleHTTPSURL(dnsName, token string) string {
	if dnsName == "" {
		return ""
	}
	return fmt.Sprintf("https://%s/?token=%s", dnsName, neturl.QueryEscape(token))
}

// handleTailscaleStatus は GET /api/mobile-connect/tailscale。
// Tailscale 自己診断結果＋ready 時の https_url＋UI 併記用の等価コマンドを返す。
func (s *Server) handleTailscaleStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	st := s.probeTailscale(r.Context())
	port := s.currentHubPort()
	token := s.snapshotCfg().Token

	resp := map[string]any{
		"state":             st.State,
		"dns_name":          st.DNSName,
		"online":            st.Online,
		"hub_port":          port,
		"serve_command":     tailscaleServeCommand(port),
		"serve_off_command": tailscaleServeOffCommand(),
	}
	if st.AdminURL != "" {
		resp["admin_url"] = st.AdminURL
	}
	if st.State == tsStateReady {
		resp["https_url"] = tailscaleHTTPSURL(st.DNSName, token)
	}

	// token を含むためキャッシュ禁止。
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, resp)
}

// handleTailscaleServe は POST /api/mobile-connect/tailscale/serve（有効化）と
// DELETE 同パス（停止 / D6）を多重化する。
func (s *Server) handleTailscaleServe(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodPost:
		s.handleTailscaleServeEnable(w, r)
	case http.MethodDelete:
		s.handleTailscaleServeDisable(w, r)
	default:
		// guard 経由でメソッド検証する（POST/DELETE 両許可）。
		if !s.guard(w, r, http.MethodPost, http.MethodDelete) {
			return
		}
	}
}

// handleTailscaleServeEnable は `tailscale serve --bg <hubPort>` を実行し、
// 成功時は Self.DNSName を allowed_hosts へ冪等追加して設定リロードする（A1）。
func (s *Server) handleTailscaleServeEnable(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost, http.MethodDelete) {
		return
	}

	st := s.enableTailscaleServe(r.Context())
	port := s.currentHubPort()
	resp := map[string]any{
		"state":    st.State,
		"dns_name": st.DNSName,
		"hub_port": port,
	}

	switch st.State {
	case tsStateNotInstalled, tsStateNotLoggedIn:
		resp["ok"] = false
		writeJSON(w, resp)
		return
	case tsStateServeDisabled:
		// tailnet で HTTPS 未有効化。管理コンソール URL を案内（A2: 500 にしない）。
		resp["ok"] = false
		if st.AdminURL != "" {
			resp["admin_url"] = st.AdminURL
		}
		writeJSON(w, resp)
		return
	case tsStateReady:
		// A1/IP-2: serve 成功 → DNS 名を allowed_hosts へ冪等追加＋リロード。
		hostAdded, err := s.addAllowedHost(st.DNSName)
		if err != nil {
			s.logger.Warn("add allowed_host failed", "err", err)
			// 自動追記不可: 案内文言に DNS 名を埋めて手動フォールバック。
			resp["ok"] = true
			resp["allowed_host_added"] = false
			resp["allowed_host_hint"] = st.DNSName
			token := s.snapshotCfg().Token
			resp["https_url"] = tailscaleHTTPSURL(st.DNSName, token)
			w.Header().Set("Cache-Control", "no-store")
			writeJSON(w, resp)
			return
		}
		resp["ok"] = true
		resp["allowed_host_added"] = hostAdded
		token := s.snapshotCfg().Token
		resp["https_url"] = tailscaleHTTPSURL(st.DNSName, token)
		// https_url は token を含むためキャッシュ禁止。
		w.Header().Set("Cache-Control", "no-store")
		writeJSON(w, resp)
		return
	default:
		// serve_inactive のまま（その他エラー）。
		resp["ok"] = false
		writeJSON(w, resp)
		return
	}
}

// handleTailscaleServeDisable は `tailscale serve --https=443 off` を実行し公開を停止する（D6）。
func (s *Server) handleTailscaleServeDisable(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost, http.MethodDelete) {
		return
	}
	if err := s.disableTailscaleServe(r.Context()); err != nil {
		s.logger.Warn("tailscale serve off failed", "err", err)
		writeJSON(w, map[string]any{"ok": false})
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

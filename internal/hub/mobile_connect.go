package hub

import (
	"fmt"
	"net/http"
	neturl "net/url"
	"os"
)

// handleMobileConnect は 📱モバイル接続ポップアップ用の接続情報を返す。
//
// フロントは token を HttpOnly cookie 化した後は JS から読めないため、
// token 入りの Hub URL はサーバー側で組み立てて返す（handleIndex の cookie 発行と
// 同等のリスク水準: guard を通過した＝token 提示済みのクライアントにのみ返す）。
//
// 経路別 URL モデル（plan_mobile-connect-flow-redesign.md C2）:
//   - SSH ローカルフォワード: スマホで ssh -L を張った後に開く 127.0.0.1 URL
//     （`hub_url` / `ssh_forward_url`）。**127.0.0.1 が正しい唯一の経路**。
//   - Tailscale（HTTPS）: 本 API では持たない。C1 の
//     `/api/mobile-connect/tailscale` エンドポイント側に集約（D3: 生IP経路は撤廃。
//     127.0.0.1 を VPN 経路の QR へ流用しない）。
//   - WireGuard（上級・接続後）: WG サーバ IP はユーザー入力依存のため、本物 URL は
//     生成できない。生IP/HTTPS の URL 雛形（テンプレート）と注記のみ返す
//     （ダミー `my-pc.example` は撤廃）。
//
// セキュリティ方針:
//   - Hub の bind は 127.0.0.1 固定のまま。SSH 経路はスマホ側のローカルフォワード前提。
//   - S2: token を含むためレスポンスは Cache-Control: no-store。
func (s *Server) handleMobileConnect(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}

	_, lanIP := hostNetInfo()
	sshUser := os.Getenv("USERNAME")
	if sshUser == "" {
		sshUser = os.Getenv("USER")
	}
	port := s.currentHubPort()
	token := s.snapshotCfg().Token

	const sshPort = 22
	sshCommand := fmt.Sprintf("ssh -L %d:127.0.0.1:%d %s@%s", port, port, sshUser, lanIP)
	sshURL := fmt.Sprintf("ssh://%s@%s:%d", sshUser, lanIP, sshPort)
	// SSH 経路でスマホが開く URL（ローカルフォワード後・127.0.0.1 が正しい唯一の経路）。
	sshForwardURL := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", port, neturl.QueryEscape(token))

	// SSH 経路: OpenSSH Server の導入/起動状態を検知（未導入なら有効化導線フラグ）。
	sshd := s.probeSSHD(r.Context())

	// WireGuard（上級・接続後）: WG サーバ IP はユーザーが知っている前提。
	// 本物 URL は生成できないため雛形と注記のみ（ダミーの my-pc.example は撤廃）。
	// `<wg-server-ip>` プレースホルダはフロントでユーザー入力に差し替える。
	wgURLTemplate := fmt.Sprintf("http://<wg-server-ip>:%d/?token=%s", port, neturl.QueryEscape(token))
	wgURLTemplateHTTPS := fmt.Sprintf("https://<wg-server-ip>/?token=%s", neturl.QueryEscape(token))

	// IP-1: 到達ホスト候補は C1 の共通リゾルバを使う（host 検出を二重実装しない）。
	// tailnet 名はこのエンドポイントでは解決しない（Tailscale は専用 EP に集約）。
	hostCandidates := reachableHosts("")

	// S2: token を含むためキャッシュ禁止。
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"lan_ip":      lanIP,
		"ssh_user":    sshUser,
		"ssh_port":    sshPort,
		"hub_port":    port,
		"ssh_command": sshCommand,
		"ssh_url":     sshURL,
		// hub_url は後方互換のため維持（= SSH 経路で開く 127.0.0.1 URL）。
		"hub_url":         sshForwardURL,
		"ssh_forward_url": sshForwardURL,
		"host_candidates": hostCandidates,

		// SSH 経路: OpenSSH Server 状態（C3 が有効化導線の出し分けに使う）。
		"sshd_state":     sshd.State,
		"sshd_installed": sshd.installed(),
		"sshd_running":   sshd.running(),

		// WireGuard（上級・接続後）の URL 雛形＋注記（ダミー撤廃）。
		"wireguard": map[string]any{
			"url_template":       wgURLTemplate,
			"url_template_https": wgURLTemplateHTTPS,
			"note":               "WireGuard サーバ IP は接続後にユーザーが指定する前提（Hub は WG 設定を持たない）。生IP は secure context 外のため通知/音声/PWA が制限される。HTTPS 化を推奨。",
		},
	})
}

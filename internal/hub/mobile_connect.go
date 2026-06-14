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
// セキュリティ方針（plan_mobile-qr-ssh-tunnel.md）:
//   - Hub の bind は 127.0.0.1 固定のまま。hub_url は 127.0.0.1 を指し、
//     スマホ側で SSH ローカルフォワード / VPN を張った前提で到達する。
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
	hubURL := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", port, neturl.QueryEscape(token))

	// S2: token を含むためキャッシュ禁止。
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, map[string]any{
		"lan_ip":      lanIP,
		"ssh_user":    sshUser,
		"ssh_port":    sshPort,
		"hub_port":    port,
		"ssh_command": sshCommand,
		"ssh_url":     sshURL,
		"hub_url":     hubURL,
	})
}

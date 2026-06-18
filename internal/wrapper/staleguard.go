package wrapper

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"time"

	"many-ai-cli/internal/config"
)

// hubInfoLite は /api/info のうち stale 判定に必要なフィールドだけを抜き出す。
type hubInfoLite struct {
	BinaryStale    bool `json:"binary_stale"`
	ActiveSessions int  `json:"active_sessions"`
}

// fetchHubInfoLite は稼働中 Hub の /api/info から stale フラグとアクティブ
// セッション数を取得する。取得・パースに失敗したら ok=false。
func fetchHubInfoLite(cfg *config.Config) (hubInfoLite, bool) {
	u := fmt.Sprintf("http://127.0.0.1:%d/api/info?token=%s", cfg.Hub.Port, url.QueryEscape(cfg.Token))
	client := &http.Client{Timeout: hubProbeTimeout}
	resp, err := client.Get(u)
	if err != nil {
		return hubInfoLite{}, false
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return hubInfoLite{}, false
	}
	var info hubInfoLite
	if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
		return hubInfoLite{}, false
	}
	return info, true
}

// maybeRestartStaleHub は「稼働中 Hub が古いバイナリか」を /api/info で確認し、
// 安全なら停止する。戻り値 true は「stale な Hub を停止したので呼び出し側は
// 新バイナリで Hub を spawn し直すべき」を意味する。false は「既存 Hub を
// そのまま使ってよい（stale でない / セッションがあり停止できない / 自動再起動
// 無効 / 停止失敗）」。
//
// プロセスは起動時に exe をメモリへ載せるため、make build で exe を差し替えても
// 走り続ける Hub は古いまま動き続ける。`many-ai-cli claude` 等を叩いた時点で
// この食い違いを検知し、巻き込むセッションが無ければ自動で載せ替える。
func maybeRestartStaleHub(cfg *config.Config, logger *slog.Logger) bool {
	info, ok := fetchHubInfoLite(cfg)
	if !ok || !info.BinaryStale {
		return false
	}
	if info.ActiveSessions > 0 {
		// 自動 kill は走行中セッションを巻き込むため行わない。手隙の再起動を促す。
		logger.Warn("running Hub is a STALE binary; rebuild won't take effect until you restart it",
			"active_sessions", info.ActiveSessions)
		return false
	}
	if !cfg.Hub.StaleBinaryAutoRestart {
		logger.Warn("running Hub is a STALE binary (auto-restart disabled); restart it manually to apply the rebuild")
		return false
	}
	logger.Info("running Hub is a stale binary with no active sessions; restarting to load the new build")
	if err := requestHubShutdown(cfg); err != nil {
		logger.Warn("failed to stop stale Hub; using it as-is", "err", err)
		return false
	}
	return true
}

// requestHubShutdown は稼働中 Hub に /api/shutdown を投げ、実際にポートが
// 応答しなくなる（プロセス終了）まで待つ。待たずに新 serve を spawn すると
// 旧 Hub がポートを掴んだまま衝突するため、停止確認を挟む。
func requestHubShutdown(cfg *config.Config) error {
	u := fmt.Sprintf("http://127.0.0.1:%d/api/shutdown?token=%s", cfg.Hub.Port, url.QueryEscape(cfg.Token))
	client := &http.Client{Timeout: hubProbeTimeout}
	resp, err := client.Post(u, "application/json", nil)
	if err != nil {
		return fmt.Errorf("post shutdown: %w", err)
	}
	_ = resp.Body.Close()
	deadline := time.Now().Add(hubStartupTimeout)
	for time.Now().Before(deadline) {
		if !probeHubAlive(cfg) {
			return nil
		}
		time.Sleep(hubStartupPoll)
	}
	return fmt.Errorf("hub did not stop within %s", hubStartupTimeout)
}

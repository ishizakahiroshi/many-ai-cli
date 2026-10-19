package hub

import (
	"net"
	"os"
	"runtime"
	"strings"

	"many-ai-cli/internal/wslutil"
)

type envMeta struct {
	Kind      string
	Label     string
	Short     string
	Color     string
	Title     string
	HostLabel string
}

var envMetaByKind = map[string]envMeta{
	"local": {
		Kind:  "local",
		Label: "Local",
		Short: "L",
		Color: "#22c55e",
		Title: "L MANY-AI-CLI",
	},
	"wsl": {
		Kind:  "wsl",
		Label: "WSL",
		Short: "W",
		Color: "#3b82f6",
		Title: "W MANY-AI-CLI",
	},
	"remote": {
		Kind:  "remote",
		Label: "Remote server",
		Short: "R",
		Color: "#f97316",
		Title: "R MANY-AI-CLI",
	},
	"remote-tunnel": {
		Kind:  "remote-tunnel",
		Label: "Remote server (tunnel)",
		Short: "T",
		Color: "#ef4444",
		Title: "T MANY-AI-CLI",
	},
}

func runtimeMode() string {
	if wslutil.IsWindowsLauncherMode() {
		return "windows-wsl"
	}
	if wslutil.IsWSL() {
		return "wsl"
	}
	if runtime.GOOS == "windows" {
		return "windows-native"
	}
	return runtime.GOOS
}

// parseSSHServerIP は SSH_CONNECTION（"クライアントIP ポート サーバIP ポート"）から
// サーバ側（自機）IP を取り出す。形式不正なら空文字を返す。
func parseSSHServerIP(conn string) string {
	fields := strings.Fields(conn)
	if len(fields) >= 3 {
		return fields[2]
	}
	return ""
}

// localIP は最初の非ループバック・非リンクローカル IPv4 アドレスを返す。
// 取得できなければ空文字（ネットワーク通信は発生しない）。
func localIP() string {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return ""
	}
	for _, a := range addrs {
		ipNet, ok := a.(*net.IPNet)
		if !ok || ipNet.IP.IsLoopback() || ipNet.IP.IsLinkLocalUnicast() {
			continue
		}
		if v4 := ipNet.IP.To4(); v4 != nil {
			return v4.String()
		}
	}
	return ""
}

// hostNetInfo はバッジ表示用の SSH セッション判定と自機 IP を返す。
// MANY_AI_CLI_HOST_LABEL があれば最優先で表示ラベルとして使う
// （launcher が SSH serve 起動時にプロファイルの host を注入する。
// コンテナ内実行等で NIC からグローバル IP を検出できないケースへの対処）。
// 次に SSH セッション経由で Hub が起動された場合は SSH_CONNECTION のサーバ側 IP、
// それ以外（systemd 常駐等）は NIC から自機 IP を取得する。
func hostNetInfo() (ssh bool, hostIP string) {
	if conn := os.Getenv("SSH_CONNECTION"); conn != "" {
		ssh = true
		hostIP = parseSSHServerIP(conn)
	} else if os.Getenv("SSH_CLIENT") != "" || os.Getenv("SSH_TTY") != "" {
		ssh = true
	}
	if label := os.Getenv("MANY_AI_CLI_HOST_LABEL"); label != "" {
		return ssh, label
	}
	if hostIP == "" {
		hostIP = localIP()
	}
	return ssh, hostIP
}

func runtimeLabel(mode string) string {
	switch mode {
	case "windows-wsl":
		return "Windows + WSL (many-ai-cli-launcher.exe)"
	case "windows-native":
		return "Windows native"
	case "wsl":
		return "WSL Linux"
	case "darwin":
		return "macOS"
	case "linux":
		return "Linux"
	default:
		return mode
	}
}

func normalizeEnvKind(kind string) string {
	switch strings.ToLower(strings.TrimSpace(kind)) {
	case "local":
		return "local"
	case "wsl":
		return "wsl"
	case "remote":
		return "remote"
	case "remote-tunnel", "remotetunnel", "remote_tunnel":
		return "remote-tunnel"
	default:
		return ""
	}
}

func resolveEnvMeta(configKind, mode string, ssh bool, hostLabel string, netHintSSH bool, netHintKind string) envMeta {
	resolveExplicit := func(raw string) (envMeta, bool) {
		if strings.TrimSpace(raw) == "" {
			return envMeta{}, false
		}
		kind := normalizeEnvKind(raw)
		if kind == "" {
			kind = "local"
		}
		return envMetaForKind(kind, hostLabel), true
	}
	if meta, ok := resolveExplicit(os.Getenv("MANY_AI_CLI_ENV_KIND")); ok {
		return meta
	}
	if meta, ok := resolveExplicit(configKind); ok {
		return meta
	}
	if meta, ok := resolveExplicit(netHintKind); ok {
		return meta
	}
	if netHintSSH {
		return envMetaForKind("remote-tunnel", hostLabel)
	}
	if mode == "windows-wsl" || mode == "wsl" {
		return envMetaForKind("wsl", hostLabel)
	}
	if ssh {
		return envMetaForKind("remote", hostLabel)
	}
	return envMetaForKind("local", hostLabel)
}

func envMetaForKind(kind, hostLabel string) envMeta {
	meta, ok := envMetaByKind[normalizeEnvKind(kind)]
	if !ok {
		meta = envMetaByKind["local"]
	}
	meta.HostLabel = strings.TrimSpace(hostLabel)
	return meta
}

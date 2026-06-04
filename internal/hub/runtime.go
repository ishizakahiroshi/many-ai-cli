package hub

import (
	"net"
	"os"
	"runtime"
	"strings"

	"any-ai-cli/internal/wslutil"
)

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
// SSH セッション経由で Hub が起動された場合は SSH_CONNECTION のサーバ側 IP を優先し、
// それ以外（systemd 常駐等）は NIC から自機 IP を取得する。
func hostNetInfo() (ssh bool, hostIP string) {
	if conn := os.Getenv("SSH_CONNECTION"); conn != "" {
		ssh = true
		hostIP = parseSSHServerIP(conn)
	} else if os.Getenv("SSH_CLIENT") != "" || os.Getenv("SSH_TTY") != "" {
		ssh = true
	}
	if hostIP == "" {
		hostIP = localIP()
	}
	return ssh, hostIP
}

func runtimeLabel(mode string) string {
	switch mode {
	case "windows-wsl":
		return "Windows + WSL (any-ai-cli-wsl.exe)"
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

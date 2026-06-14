package launcher

// export.go — `many-ai-cli profile-export` が吐く自己記述プロファイルの生成。
//
// リモートサーバー上で実行し、自分へ SSH 接続するための Profile（serve モード）を
// 鍵を除いて JSON 出力する。手元 PC の UI は SSH-pull でこれを取得し、接続
// プロファイル追加フォームを自動補完する（plan_server-profile-export-import.md C1）。
//
// サーバーが埋められない値はクライアントが補完する:
//   - IdentityFile（鍵）— クライアント側のパス。export には含めない。
//   - 到達ホスト — 自己検出 IP は Docker ではコンテナ内 IP になり手元から届かない。
//     HostCandidates を配列で吐き、--host / MANY_AI_CLI_PUBLIC_HOST で公開ホストを
//     先頭に上書きできる。クライアントは実際に SSH した host を既定採用する。

import (
	"net"
	"os"
	"strings"
)

// ExportedProfile は `profile-export --json` の出力ペイロード。
// Profile は鍵を除いた接続プロファイル本体、HostCandidates は到達ホスト候補。
type ExportedProfile struct {
	Profile        Profile  `json:"profile"`
	HostCandidates []string `json:"host_candidates"`
}

// ExportOptions は自動検出値を上書きする export 用オプション。
type ExportOptions struct {
	Name       string // 表示名（既定はホスト名ベース）
	PublicHost string // 公開ホスト上書き（Docker のコンテナ内 IP 対策。先頭候補になる）
	CWD        string // リモート作業ディレクトリ（既定は実行時 cwd）
	HubPort    int    // 0 = auto-select
}

// BuildExportProfile は自己情報から serve モードの SSH Profile を組み立てる。
// IdentityFile（鍵）は常に空（取り込み側クライアントが補完する）。
func BuildExportProfile(opts ExportOptions) ExportedProfile {
	user := os.Getenv("USERNAME")
	if user == "" {
		user = os.Getenv("USER")
	}

	candidates := exportHostCandidates(opts.PublicHost)

	cwd := strings.TrimSpace(opts.CWD)
	if cwd == "" {
		if wd, err := os.Getwd(); err == nil {
			cwd = wd
		}
	}

	host := ""
	if len(candidates) > 0 {
		host = candidates[0]
	}

	name := strings.TrimSpace(opts.Name)
	if name == "" {
		name = defaultExportName(host)
	}

	p := Profile{
		Name:    name,
		Type:    ProfileTypeSSH,
		Mode:    SSHModeServe,
		Host:    host,
		User:    user,
		Binary:  exportBinaryName(),
		CWD:     cwd,
		HubPort: opts.HubPort,
	}
	return ExportedProfile{Profile: p, HostCandidates: candidates}
}

// defaultExportName は表示名の既定値（ホスト名 → 先頭候補 → "remote"）を返す。
func defaultExportName(host string) string {
	if hn, err := os.Hostname(); err == nil {
		if hn = strings.TrimSpace(hn); hn != "" {
			return hn
		}
	}
	if host != "" {
		return host
	}
	return "remote"
}

// exportBinaryName は os.Executable() の basename を返す（解決できなければ
// "many-ai-cli"）。Windows の .exe 拡張子はそのまま残す。
func exportBinaryName() string {
	exe, err := os.Executable()
	if err != nil {
		return "many-ai-cli"
	}
	base := exe
	if i := strings.LastIndexAny(base, `/\`); i >= 0 {
		base = base[i+1:]
	}
	base = strings.TrimSpace(base)
	if base == "" {
		return "many-ai-cli"
	}
	return base
}

// exportHostCandidates は到達ホスト候補を優先度順・重複排除して返す。
// 並び: 明示 --host → MANY_AI_CLI_PUBLIC_HOST → SSH_CONNECTION のサーバ側 IP →
// NIC の非ループバック IPv4 → os.Hostname()。
func exportHostCandidates(publicHost string) []string {
	var out []string
	seen := map[string]struct{}{}
	add := func(h string) {
		h = strings.TrimSuffix(strings.TrimSpace(h), ".")
		if h == "" {
			return
		}
		key := strings.ToLower(h)
		if _, ok := seen[key]; ok {
			return
		}
		seen[key] = struct{}{}
		out = append(out, h)
	}

	add(publicHost)
	add(os.Getenv("MANY_AI_CLI_PUBLIC_HOST"))
	add(sshConnectionServerIP())
	for _, ip := range exportLANIPs() {
		add(ip)
	}
	if hn, err := os.Hostname(); err == nil {
		add(hn)
	}
	return out
}

// sshConnectionServerIP は SSH 経由で起動された場合の SSH_CONNECTION サーバ側 IP
// （3 番目のフィールド）を返す。未設定なら空。
func sshConnectionServerIP() string {
	fields := strings.Fields(os.Getenv("SSH_CONNECTION"))
	if len(fields) >= 3 {
		return fields[2]
	}
	return ""
}

// exportLANIPs は非ループバック・非リンクローカルな IPv4 アドレスを列挙する。
// 取得できなければ空（ネットワーク通信は発生しない）。
func exportLANIPs() []string {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return nil
	}
	var out []string
	for _, a := range addrs {
		ipNet, ok := a.(*net.IPNet)
		if !ok || ipNet.IP.IsLoopback() || ipNet.IP.IsLinkLocalUnicast() {
			continue
		}
		if v4 := ipNet.IP.To4(); v4 != nil {
			out = append(out, v4.String())
		}
	}
	return out
}

package hub

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// Tailscale 自己診断＋serve 自動化（plan_mobile-connect-flow-redesign.md C1）。
//
// Hub が tailscale CLI を叩いて自分側の状態を診断し、tailscale serve（HTTPS・
// loopback プロキシ）で「スキャンすれば実際に繋がる本物の URL」を生成できるようにする。
//
// セキュリティ方針:
//   - Hub の bind は 127.0.0.1 固定のまま（serve が loopback へプロキシ）。Funnel は使わない。
//   - token は標準出力にもログにも残さない（CLI には渡さない）。
//   - CLI 実行は exec.CommandContext で timeout 付き（5s 目安）。

// Tailscale 状態機械の値。
const (
	tsStateNotInstalled  = "not_installed"
	tsStateNotLoggedIn   = "not_logged_in"
	tsStateServeDisabled = "serve_disabled_on_tailnet"
	tsStateServeInactive = "serve_inactive"
	tsStateReady         = "ready"
)

const tailscaleCmdTimeout = 5 * time.Second

// tailscaleRunner は tailscale CLI 実行を抽象化する（テストでモック注入する）。
// path は tailscaleExe() の結果、args は CLI 引数。stdout/stderr/err を返す。
type tailscaleRunner interface {
	// available は tailscale CLI が実行時到達可能なら true を返す。
	available() bool
	// run は tailscale CLI を timeout 付きで実行し stdout/stderr を返す。
	run(ctx context.Context, args ...string) (stdout string, stderr string, err error)
}

// execTailscaleRunner は本番用の tailscaleRunner（exec ベース）。
type execTailscaleRunner struct{}

func (execTailscaleRunner) available() bool { return tailscaleExe() != "" }

func (execTailscaleRunner) run(ctx context.Context, args ...string) (string, string, error) {
	exe := tailscaleExe()
	if exe == "" {
		return "", "", fmt.Errorf("tailscale CLI not found")
	}
	cmd := exec.CommandContext(ctx, exe, args...)
	var stdout, stderr strings.Builder
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	return stdout.String(), stderr.String(), err
}

// tailscaleRunnerFor は Server に注入された runner、未設定なら exec 実装を返す。
func (s *Server) tailscaleRunnerFor() tailscaleRunner {
	if s.tsRunner != nil {
		return s.tsRunner
	}
	return execTailscaleRunner{}
}

// tailscaleStatusJSON は `tailscale status --json` の必要フィールドのみを写す。
type tailscaleStatusJSON struct {
	BackendState string `json:"BackendState"`
	Self         struct {
		DNSName string `json:"DNSName"`
		Online  bool   `json:"Online"`
	} `json:"Self"`
}

// tailscaleState は Hub の Tailscale 自己診断結果。
type tailscaleState struct {
	State        string // 状態機械の値（tsState*）
	DNSName      string // Self.DNSName（末尾ドット除去）
	Online       bool   // Self.Online
	BackendState string // 生の BackendState（診断用）
	AdminURL     string // serve_disabled_on_tailnet 時の管理コンソール URL
}

// adminURLRe は serve 有効化失敗時の stderr から管理コンソール URL を抽出する。
var adminURLRe = regexp.MustCompile(`https://login\.tailscale\.com/\S+`)

// trimDNSName は tailscale の DNSName 末尾ドットを除去する。
func trimDNSName(name string) string {
	return strings.TrimSuffix(strings.TrimSpace(name), ".")
}

// probeTailscale は tailscale CLI を叩いて状態機械を算出する。
// CLI 不在・到達不可なら not_installed で degrade する（A2: 500 にしない）。
func (s *Server) probeTailscale(ctx context.Context) tailscaleState {
	runner := s.tailscaleRunnerFor()
	if !runner.available() {
		return tailscaleState{State: tsStateNotInstalled}
	}

	statusCtx, cancel := context.WithTimeout(ctx, tailscaleCmdTimeout)
	defer cancel()
	stdout, _, err := runner.run(statusCtx, "status", "--json")
	if err != nil {
		// CLI はあるが実行不可（headless/Docker 等）→ degrade。
		return tailscaleState{State: tsStateNotInstalled}
	}

	var st tailscaleStatusJSON
	if jsonErr := json.Unmarshal([]byte(stdout), &st); jsonErr != nil {
		return tailscaleState{State: tsStateNotInstalled}
	}

	res := tailscaleState{
		DNSName:      trimDNSName(st.Self.DNSName),
		Online:       st.Self.Online,
		BackendState: st.BackendState,
	}
	if !strings.EqualFold(st.BackendState, "Running") {
		res.State = tsStateNotLoggedIn
		return res
	}

	// serve がハブポートへ active proxy しているか判定。
	if s.tailscaleServeActive(ctx, runner) {
		res.State = tsStateReady
	} else {
		res.State = tsStateServeInactive
	}
	return res
}

// tailscaleServeActive は `tailscale serve status` を見てハブポートへの
// active proxy（proxy http://127.0.0.1:<port>）が存在するか判定する。
func (s *Server) tailscaleServeActive(ctx context.Context, runner tailscaleRunner) bool {
	serveCtx, cancel := context.WithTimeout(ctx, tailscaleCmdTimeout)
	defer cancel()
	stdout, _, err := runner.run(serveCtx, "serve", "status")
	if err != nil {
		return false
	}
	port := s.currentHubPort()
	needle := fmt.Sprintf("127.0.0.1:%d", port)
	return strings.Contains(stdout, needle)
}

// enableTailscaleServe は `tailscale serve --bg <hubPort>` を実行する。
// 成功時は state=ready 相当・DNSName を返す。tailnet 未有効化なら
// serve_disabled_on_tailnet＋管理コンソール URL を返す（A2 と同じく 500 にしない）。
func (s *Server) enableTailscaleServe(ctx context.Context) tailscaleState {
	runner := s.tailscaleRunnerFor()
	// まず status で DNSName / 前提状態を取る（degrade 判定込み）。
	st := s.probeTailscale(ctx)
	switch st.State {
	case tsStateNotInstalled, tsStateNotLoggedIn:
		// serve を打てる状態ではない。そのまま返す。
		return st
	}

	port := s.currentHubPort()
	serveCtx, cancel := context.WithTimeout(ctx, tailscaleCmdTimeout)
	defer cancel()
	_, stderr, err := runner.run(serveCtx, "serve", "--bg", strconv.Itoa(port))
	if err != nil {
		// 「Serve is not enabled on your tailnet」→ admin URL を抽出して案内。
		if admin := adminURLRe.FindString(stderr); admin != "" || strings.Contains(strings.ToLower(stderr), "not enabled") {
			st.State = tsStateServeDisabled
			st.AdminURL = admin
			return st
		}
		// その他のエラーは serve_inactive のまま（呼び出し側がエラー詳細を扱う）。
		st.State = tsStateServeInactive
		return st
	}
	st.State = tsStateReady
	return st
}

// disableTailscaleServe は `tailscale serve --https=443 off` を実行し公開を停止する（D6）。
func (s *Server) disableTailscaleServe(ctx context.Context) error {
	runner := s.tailscaleRunnerFor()
	if !runner.available() {
		return fmt.Errorf("tailscale CLI not found")
	}
	offCtx, cancel := context.WithTimeout(ctx, tailscaleCmdTimeout)
	defer cancel()
	_, stderr, err := runner.run(offCtx, "serve", "--https=443", "off")
	if err != nil {
		return fmt.Errorf("tailscale serve off: %w: %s", err, strings.TrimSpace(stderr))
	}
	return nil
}

// tailscaleServeCommand / tailscaleServeOffCommand は UI 併記用の等価コマンド文字列（D1）。
func tailscaleServeCommand(port int) string {
	return fmt.Sprintf("tailscale serve --bg %d", port)
}

func tailscaleServeOffCommand() string {
	return "tailscale serve --https=443 off"
}

// ---- IP-1: 自己ホスト解決の共通リゾルバ ----

// reachableHosts は Hub への「到達ホスト候補」集合を返す共通リゾルバ。
// 既存 hostNetInfo()（LAN IP / hostname）と MANY_AI_CLI_PUBLIC_HOST、
// および任意で渡された tailnet DNS 名を 1 つの集合にまとめる。
// mobile-connect / profile-export が同実体を使うことで host 検出を二重実装しない（IP-1）。
// 重複・空文字は除去する（順序は安定: tailnet → public host → LAN IP → hostname）。
func reachableHosts(tailnetDNS string) []string {
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

	add(tailnetDNS)
	add(os.Getenv("MANY_AI_CLI_PUBLIC_HOST"))
	if _, lanIP := hostNetInfo(); lanIP != "" {
		add(lanIP)
	}
	if hn, err := os.Hostname(); err == nil {
		add(hn)
	}
	return out
}

// ---- A1 / IP-2: allowed_hosts 冪等追加（汎用ヘルパ・モバイル専用にしない） ----

// addAllowedHost は host を hub.allowed_hosts へ冪等追加し設定を永続化する。
// 既に登録済み（大小無視・末尾ドット無視）なら何もせず added=false を返す。
// host が空なら no-op。リモート Hub を Tailscale 直で開く用途にも再利用できる汎用ヘルパ（IP-2）。
func (s *Server) addAllowedHost(host string) (added bool, err error) {
	host = strings.TrimSuffix(strings.TrimSpace(host), ".")
	if host == "" {
		return false, nil
	}
	s.cfgMu.Lock()
	for _, existing := range s.cfg.Hub.AllowedHosts {
		if strings.EqualFold(strings.TrimSuffix(strings.TrimSpace(existing), "."), host) {
			s.cfgMu.Unlock()
			return false, nil
		}
	}
	s.cfg.Hub.AllowedHosts = append(s.cfg.Hub.AllowedHosts, host)
	s.cfgMu.Unlock()

	if err := s.persistConfig(); err != nil {
		return true, fmt.Errorf("persist allowed_hosts: %w", err)
	}
	return true, nil
}

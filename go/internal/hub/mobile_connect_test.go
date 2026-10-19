package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// fakeTailscaleRunner は tailscaleRunner のモック。args 先頭で応答を出し分ける。
type fakeTailscaleRunner struct {
	avail      bool
	statusJSON string
	statusErr  error
	serveOut   string // `serve status` の stdout
	serveBgErr error  // `serve --bg` のエラー
	serveBgOut string // `serve --bg` の stderr（admin URL 抽出用）
	offErr     error
	calls      []string // 実行された引数列（token 非露出の検証用）
}

// fakeExitErr は CLI 非ゼロ終了を模した error。
type fakeExitErr struct{}

func (fakeExitErr) Error() string { return "exit status 1" }

func (f *fakeTailscaleRunner) available() bool { return f.avail }

func (f *fakeTailscaleRunner) run(_ context.Context, args ...string) (string, string, error) {
	f.calls = append(f.calls, strings.Join(args, " "))
	switch {
	case len(args) >= 2 && args[0] == "status":
		return f.statusJSON, "", f.statusErr
	case len(args) >= 2 && args[0] == "serve" && args[1] == "status":
		return f.serveOut, "", nil
	case len(args) >= 2 && args[0] == "serve" && args[1] == "--bg":
		return "", f.serveBgOut, f.serveBgErr
	case len(args) >= 2 && args[0] == "serve" && args[1] == "--https=443":
		return "", "", f.offErr
	}
	return "", "", nil
}

// callMobileConnect は /api/mobile-connect を token 付き GET で呼び、レスポンスを返す。
func callMobileConnect(t *testing.T, s *Server) (*httptest.ResponseRecorder, map[string]any) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/mobile-connect?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	var resp map[string]any
	if w.Code == http.StatusOK {
		if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
			t.Fatalf("decode /api/mobile-connect: %v", err)
		}
	}
	return w, resp
}

func TestMobileConnect_ReturnsConnectionInfo(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	w, resp := callMobileConnect(t, s)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}

	// S2: token を含むためキャッシュ禁止。
	if cc := w.Header().Get("Cache-Control"); cc != "no-store" {
		t.Errorf("Cache-Control = %q, want no-store", cc)
	}

	// hub_url は 127.0.0.1（bind 不変）を指し token を含む。
	hubURL, _ := resp["hub_url"].(string)
	if !strings.Contains(hubURL, "127.0.0.1:47777") {
		t.Errorf("hub_url = %q, want to contain 127.0.0.1:47777", hubURL)
	}
	if !strings.Contains(hubURL, "token=tok") {
		t.Errorf("hub_url = %q, want to contain token=tok", hubURL)
	}

	// ssh_command は 127.0.0.1 へのローカルフォワード。
	sshCmd, _ := resp["ssh_command"].(string)
	if !strings.HasPrefix(sshCmd, "ssh -L 47777:127.0.0.1:47777 ") {
		t.Errorf("ssh_command = %q, want prefix 'ssh -L 47777:127.0.0.1:47777 '", sshCmd)
	}

	if got, ok := resp["hub_port"].(float64); !ok || int(got) != 47777 {
		t.Errorf("hub_port = %v, want 47777", resp["hub_port"])
	}
	if got, ok := resp["ssh_port"].(float64); !ok || int(got) != 22 {
		t.Errorf("ssh_port = %v, want 22", resp["ssh_port"])
	}
	if _, ok := resp["lan_ip"].(string); !ok {
		t.Errorf("lan_ip missing or not a string: %v", resp["lan_ip"])
	}
}

// fakeSSHDProber は sshdProber のモック。固定の state を返す。
type fakeSSHDProber struct{ state string }

func (f fakeSSHDProber) probe(context.Context) sshdState { return sshdState{State: f.state} }

// C2: SSH 経路の 127.0.0.1 URL（ssh_forward_url）が正しい唯一の経路として返る。
func TestMobileConnect_SSHRouteURLModel(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.sshdProber = fakeSSHDProber{state: sshdStateRunning}

	_, resp := callMobileConnect(t, s)

	fwd, _ := resp["ssh_forward_url"].(string)
	if !strings.Contains(fwd, "127.0.0.1:47777") || !strings.Contains(fwd, "token=tok") {
		t.Errorf("ssh_forward_url = %q, want 127.0.0.1:47777 with token", fwd)
	}
	// hub_url は後方互換で SSH 経路 URL と一致。
	if resp["hub_url"] != fwd {
		t.Errorf("hub_url = %v, want equal to ssh_forward_url %q", resp["hub_url"], fwd)
	}
}

// C2/D3: VPN 経路の接続 URL に 127.0.0.1 を流用しない。
// 127.0.0.1 は SSH 経路（hub_url / ssh_forward_url / ssh_command）のみに現れる。
func TestMobileConnect_No127ForVPNRoutes(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.sshdProber = fakeSSHDProber{state: sshdStateRunning}

	_, resp := callMobileConnect(t, s)

	sshOnlyKeys := map[string]struct{}{
		"hub_url":         {},
		"ssh_forward_url": {},
		"ssh_command":     {},
	}
	for k, v := range resp {
		str, ok := v.(string)
		if !ok {
			continue
		}
		if strings.Contains(str, "127.0.0.1") {
			if _, allowed := sshOnlyKeys[k]; !allowed {
				t.Errorf("127.0.0.1 leaked into non-SSH field %q = %q", k, str)
			}
		}
	}

	// WireGuard 雛形にダミー my-pc.example が残っていないこと。
	wg, _ := resp["wireguard"].(map[string]any)
	for _, key := range []string{"url_template", "url_template_https"} {
		v, _ := wg[key].(string)
		if strings.Contains(v, "my-pc.example") || strings.Contains(v, "127.0.0.1") {
			t.Errorf("wireguard.%s = %q, want no dummy/127.0.0.1 (got placeholder <wg-server-ip>)", key, v)
		}
		if !strings.Contains(v, "<wg-server-ip>") {
			t.Errorf("wireguard.%s = %q, want <wg-server-ip> placeholder", key, v)
		}
	}
}

// C2: OpenSSH 未導入で sshd_installed=false・状態 not_installed。
func TestMobileConnect_SSHDNotInstalled(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.sshdProber = fakeSSHDProber{state: sshdStateNotInstalled}

	w, resp := callMobileConnect(t, s)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200 (degrade, not 500)", w.Code)
	}
	if resp["sshd_state"] != sshdStateNotInstalled {
		t.Errorf("sshd_state = %v, want %s", resp["sshd_state"], sshdStateNotInstalled)
	}
	if resp["sshd_installed"] != false {
		t.Errorf("sshd_installed = %v, want false", resp["sshd_installed"])
	}
	if resp["sshd_running"] != false {
		t.Errorf("sshd_running = %v, want false", resp["sshd_running"])
	}
}

// C2: OpenSSH 導入済み・起動中で sshd_installed=true・sshd_running=true。
func TestMobileConnect_SSHDRunning(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.sshdProber = fakeSSHDProber{state: sshdStateRunning}

	_, resp := callMobileConnect(t, s)
	if resp["sshd_state"] != sshdStateRunning {
		t.Errorf("sshd_state = %v, want %s", resp["sshd_state"], sshdStateRunning)
	}
	if resp["sshd_installed"] != true {
		t.Errorf("sshd_installed = %v, want true", resp["sshd_installed"])
	}
	if resp["sshd_running"] != true {
		t.Errorf("sshd_running = %v, want true", resp["sshd_running"])
	}
}

// C2: 導入済み・停止中なら installed=true / running=false（有効化導線は出さず起動導線）。
func TestMobileConnect_SSHDStopped(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.sshdProber = fakeSSHDProber{state: sshdStateStopped}

	_, resp := callMobileConnect(t, s)
	if resp["sshd_installed"] != true {
		t.Errorf("sshd_installed = %v, want true (stopped is installed)", resp["sshd_installed"])
	}
	if resp["sshd_running"] != false {
		t.Errorf("sshd_running = %v, want false", resp["sshd_running"])
	}
}

func TestMobileConnect_RejectsMissingToken(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	req := httptest.NewRequest(http.MethodGet, "/api/mobile-connect", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("code = %d, want 401", w.Code)
	}
}

func TestMobileConnect_RejectsPost(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	req := httptest.NewRequest(http.MethodPost, "/api/mobile-connect?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleMobileConnect(w, req)
	if w.Code != http.StatusMethodNotAllowed {
		t.Fatalf("code = %d, want 405", w.Code)
	}
}

// callTailscaleStatus は GET /api/mobile-connect/tailscale を token 付きで呼ぶ。
func callTailscaleStatus(t *testing.T, s *Server) (*httptest.ResponseRecorder, map[string]any) {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/mobile-connect/tailscale?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleTailscaleStatus(w, req)
	var resp map[string]any
	if w.Code == http.StatusOK {
		if err := json.NewDecoder(w.Body).Decode(&resp); err != nil {
			t.Fatalf("decode tailscale status: %v", err)
		}
	}
	return w, resp
}

const tsRunningStatus = `{"BackendState":"Running","Self":{"DNSName":"barikatavm1.taila5e951.ts.net.","Online":true}}`

// A2: CLI 不在なら not_installed に degrade し 500 にしない。
func TestMobileConnectTailscale_DegradeWhenNotInstalled(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{avail: false}

	w, resp := callTailscaleStatus(t, s)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200 (degrade, not 500)", w.Code)
	}
	if resp["state"] != tsStateNotInstalled {
		t.Errorf("state = %v, want %s", resp["state"], tsStateNotInstalled)
	}
	if cc := w.Header().Get("Cache-Control"); cc != "no-store" {
		t.Errorf("Cache-Control = %q, want no-store", cc)
	}
}

// status の status JSON が壊れていても not_installed へ degrade（500 にしない）。
func TestMobileConnectTailscale_DegradeOnRunError(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{avail: true, statusJSON: "garbage{"}

	w, resp := callTailscaleStatus(t, s)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}
	if resp["state"] != tsStateNotInstalled {
		t.Errorf("state = %v, want %s", resp["state"], tsStateNotInstalled)
	}
}

// BackendState != Running なら not_logged_in。
func TestMobileConnectTailscale_NotLoggedIn(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{
		avail:      true,
		statusJSON: `{"BackendState":"NeedsLogin","Self":{"DNSName":"x.ts.net.","Online":false}}`,
	}

	_, resp := callTailscaleStatus(t, s)
	if resp["state"] != tsStateNotLoggedIn {
		t.Errorf("state = %v, want %s", resp["state"], tsStateNotLoggedIn)
	}
}

// Running かつ serve がハブポートへ active proxy なら ready＋本物 https_url（token 入り）。
func TestMobileConnectTailscale_ReadyURL(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{
		avail:      true,
		statusJSON: tsRunningStatus,
		serveOut:   "https://barikatavm1.taila5e951.ts.net (tailnet only)\n|-- proxy http://127.0.0.1:47777",
	}

	w, resp := callTailscaleStatus(t, s)
	if resp["state"] != tsStateReady {
		t.Fatalf("state = %v, want %s", resp["state"], tsStateReady)
	}
	httpsURL, _ := resp["https_url"].(string)
	// 末尾ドットが除去されていること。
	if !strings.HasPrefix(httpsURL, "https://barikatavm1.taila5e951.ts.net/") {
		t.Errorf("https_url = %q, want https://<dns>/ prefix without trailing dot", httpsURL)
	}
	if !strings.Contains(httpsURL, "token=tok") {
		t.Errorf("https_url = %q, want token=tok", httpsURL)
	}
	// 等価コマンドが併記されること（D1）。
	if cmd, _ := resp["serve_command"].(string); cmd != "tailscale serve --bg 47777" {
		t.Errorf("serve_command = %q", cmd)
	}
	if cc := w.Header().Get("Cache-Control"); cc != "no-store" {
		t.Errorf("Cache-Control = %q, want no-store", cc)
	}
}

// token は CLI 引数として渡されない（標準出力/ログ非露出の代替検証）。
func TestMobileConnectTailscale_TokenNotPassedToCLI(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "supersecret"
	runner := &fakeTailscaleRunner{
		avail:      true,
		statusJSON: tsRunningStatus,
		serveOut:   "proxy http://127.0.0.1:47777",
	}
	s.tsRunner = runner

	callTailscaleStatus(t, s)
	for _, c := range runner.calls {
		if strings.Contains(c, "supersecret") {
			t.Errorf("token leaked into CLI args: %q", c)
		}
	}
}

// A1: serve 有効化で Self.DNSName が allowed_hosts へ冪等追加される（2回呼んでも重複しない）。
func TestMobileConnectTailscale_ServeAddsAllowedHostIdempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{
		avail:      true,
		statusJSON: tsRunningStatus,
		serveOut:   "proxy http://127.0.0.1:47777",
	}

	enable := func() map[string]any {
		req := httptest.NewRequest(http.MethodPost, "/api/mobile-connect/tailscale/serve?token=tok", nil)
		req.Host = "127.0.0.1:47777"
		req.Header.Set("Origin", "http://127.0.0.1:47777")
		w := httptest.NewRecorder()
		s.handleTailscaleServe(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("serve enable code = %d, want 200", w.Code)
		}
		var resp map[string]any
		_ = json.NewDecoder(w.Body).Decode(&resp)
		return resp
	}

	resp1 := enable()
	if resp1["ok"] != true {
		t.Fatalf("first enable ok = %v, want true (resp=%v)", resp1["ok"], resp1)
	}
	resp2 := enable()
	if resp2["ok"] != true {
		t.Fatalf("second enable ok = %v, want true", resp2["ok"])
	}

	const want = "barikatavm1.taila5e951.ts.net"
	count := 0
	for _, h := range s.cfg.Hub.AllowedHosts {
		if h == want {
			count++
		}
	}
	if count != 1 {
		t.Errorf("allowed_hosts contains %q %d times, want exactly 1 (got %v)", want, count, s.cfg.Hub.AllowedHosts)
	}
}

// serve_disabled_on_tailnet 時は admin URL を抽出して返す（500 にしない）。
func TestMobileConnectTailscale_ServeDisabledReturnsAdminURL(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Token = "tok"
	s.tsRunner = &fakeTailscaleRunner{
		avail:      true,
		statusJSON: tsRunningStatus,
		serveOut:   "", // serve inactive
		serveBgErr: &fakeExitErr{},
		serveBgOut: "error: Serve is not enabled on your tailnet. To enable, visit:\n  https://login.tailscale.com/f/serve?node=abc123\n",
	}

	req := httptest.NewRequest(http.MethodPost, "/api/mobile-connect/tailscale/serve?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	w := httptest.NewRecorder()
	s.handleTailscaleServe(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200 (not 500)", w.Code)
	}
	var resp map[string]any
	_ = json.NewDecoder(w.Body).Decode(&resp)
	if resp["state"] != tsStateServeDisabled {
		t.Errorf("state = %v, want %s", resp["state"], tsStateServeDisabled)
	}
	admin, _ := resp["admin_url"].(string)
	if !strings.HasPrefix(admin, "https://login.tailscale.com/f/serve?") {
		t.Errorf("admin_url = %q, want login.tailscale.com URL", admin)
	}
}

// DELETE …/serve は公開停止コマンドを実行する（D6）。
func TestMobileConnectTailscale_ServeDisableOff(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	runner := &fakeTailscaleRunner{avail: true}
	s.tsRunner = runner

	req := httptest.NewRequest(http.MethodDelete, "/api/mobile-connect/tailscale/serve?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	w := httptest.NewRecorder()
	s.handleTailscaleServe(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200", w.Code)
	}
	var resp map[string]any
	_ = json.NewDecoder(w.Body).Decode(&resp)
	if resp["ok"] != true {
		t.Errorf("ok = %v, want true", resp["ok"])
	}
	sawOff := false
	for _, c := range runner.calls {
		if strings.Contains(c, "serve --https=443 off") {
			sawOff = true
		}
	}
	if !sawOff {
		t.Errorf("expected serve off command, calls=%v", runner.calls)
	}
}

// addAllowedHost は単体でも冪等（大小・末尾ドット無視）。
func TestMobileConnectTailscale_AddAllowedHostHelperIdempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.Hub.AllowedHosts = []string{"Host-A.ts.net"}

	added, err := s.addAllowedHost("host-a.ts.net.")
	if err != nil {
		t.Fatalf("addAllowedHost err: %v", err)
	}
	if added {
		t.Errorf("added = true, want false (already present, case/dot-insensitive)")
	}

	added, err = s.addAllowedHost("new-host.ts.net")
	if err != nil {
		t.Fatalf("addAllowedHost err: %v", err)
	}
	if !added {
		t.Errorf("added = false, want true for new host")
	}
}

// reachableHosts は tailnet 名・LAN IP・hostname を重複なくまとめる（IP-1）。
func TestMobileConnectTailscale_ReachableHostsDedup(t *testing.T) {
	t.Setenv("MANY_AI_CLI_PUBLIC_HOST", "pub.example.com")
	hosts := reachableHosts("my-pc.ts.net.")
	if len(hosts) == 0 || hosts[0] != "my-pc.ts.net" {
		t.Fatalf("hosts[0] = %v, want my-pc.ts.net (trailing dot trimmed)", hosts)
	}
	seen := map[string]int{}
	for _, h := range hosts {
		seen[strings.ToLower(h)]++
	}
	for h, n := range seen {
		if n != 1 {
			t.Errorf("host %q appears %d times, want 1", h, n)
		}
	}
	foundPub := false
	for _, h := range hosts {
		if h == "pub.example.com" {
			foundPub = true
		}
	}
	if !foundPub {
		t.Errorf("MANY_AI_CLI_PUBLIC_HOST not included: %v", hosts)
	}
}

// TestDocumentCSP_NoAllowedHosts は allowed_hosts 未設定なら loopback のみの
// connect-src で、VPN-IP のソースが含まれないことを確認する（C5）。
func TestDocumentCSP_NoAllowedHosts(t *testing.T) {
	s := newTestServer()
	csp := s.documentCSP()
	if !strings.Contains(csp, "connect-src 'self' ws://127.0.0.1:* ws://localhost:*") {
		t.Errorf("csp connect-src base missing: %q", csp)
	}
	if strings.Contains(csp, "100.101.102.103") {
		t.Errorf("csp unexpectedly contains a VPN host: %q", csp)
	}
}

// TestDocumentCSP_AllowedHostExpands は allowed_hosts の host が
// ws:// と wss:// の両方で connect-src に展開されることを確認する（C5 / G2）。
func TestDocumentCSP_AllowedHostExpands(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowedHosts = []string{"100.101.102.103", "my-pc.tailnet.ts.net"}
	csp := s.documentCSP()
	for _, want := range []string{
		"ws://100.101.102.103:*",
		"wss://100.101.102.103:*",
		"ws://my-pc.tailnet.ts.net:*",
		"wss://my-pc.tailnet.ts.net:*",
	} {
		if !strings.Contains(csp, want) {
			t.Errorf("csp missing %q in %q", want, csp)
		}
	}
}

package launcher

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"regexp"
	"strings"
	"sync"
	"time"
	"unicode"
)

// SSHConnector implements Connector for SSH profiles (serve and tunnel modes).
//
// out controls where the scanned child-process output is mirrored. nil keeps
// the launcher-exe behaviour (os.Stdout / os.Stderr — the console the user
// watches). The Hub sets it to io.Discard via setQuiet so hosting a tunnel
// produces no console noise and never echoes the remote Hub URL + token.
type SSHConnector struct {
	out io.Writer
}

func (c *SSHConnector) stdoutWriter() io.Writer {
	if c.out != nil {
		return c.out
	}
	return os.Stdout
}

func (c *SSHConnector) stderrWriter() io.Writer {
	if c.out != nil {
		return c.out
	}
	return os.Stderr
}

// setQuiet implements the quietable interface used by ConnectorForQuiet.
func (c *SSHConnector) setQuiet(w io.Writer) { c.out = w }

// Start launches the SSH connection described by p.
// For serve mode it starts `ssh -t -L …` which runs `many-ai-cli serve`
// on the remote host. For tunnel mode it forwards the port of an already-
// running remote Hub and fetches the token via token_command.
func (c *SSHConnector) Start(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) error {
	if p.Host == "" {
		return fmt.Errorf("ssh profile %q: host is required", p.Name)
	}
	mode := p.Mode
	if mode == "" {
		mode = SSHModeServe
	}

	switch mode {
	case SSHModeServe:
		go c.runServe(ctx, p, urlCh, errCh)
	case SSHModeTunnel:
		go c.runTunnel(ctx, p, urlCh, errCh)
	default:
		return fmt.Errorf("ssh profile %q: unknown mode %q", p.Name, mode)
	}
	return nil
}

// --------------------------------------------------------------------------
// serve mode
// --------------------------------------------------------------------------

const (
	sshMaxRetries = 5
	sshPortStep   = 100
)

func (c *SSHConnector) runServe(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) {
	binary := p.Binary
	if binary == "" {
		binary = "many-ai-cli"
	}

	basePort := p.HubPort
	if basePort == 0 {
		basePort = PickPort()
	}

	var lastErr error
	for attempt := 0; attempt < sshMaxRetries; attempt++ {
		port := basePort + attempt*sshPortStep

		url, cmd, waitCh, err := c.tryServe(ctx, p, binary, port)
		if err != nil {
			lastErr = err
			if ctx.Err() != nil {
				break
			}
			continue
		}

		// URL obtained successfully. Send it, then keep the ssh process alive
		// until it exits or ctx is cancelled; clean up remote serve on exit.
		select {
		case urlCh <- url:
		case <-ctx.Done():
		}
		close(urlCh)

		// ssh の終了（= リモート serve の停止。Web UI の「Web のみ停止」
		// を含む）か ctx キャンセルを待つ。errCh の close は「接続終了」の
		// 合図で、launcher 本体はこれを受けてプロセスを終了する
		//（コンソール窓の残骸防止）。
		select {
		case <-waitCh:
		case <-ctx.Done():
			_ = cmd.Process.Kill()
			<-waitCh
		}
		cleanupSSHOrphans(p, binary, port)
		close(errCh)
		return
	}

	if lastErr == nil {
		lastErr = fmt.Errorf("ssh serve: max retries exceeded")
	}
	select {
	case errCh <- lastErr:
	case <-ctx.Done():
	}
	close(urlCh)
	close(errCh)
}

// tryServe starts ssh and blocks until the Hub URL is detected or an error
// occurs. On success it returns the URL, the live *exec.Cmd (which the caller
// must eventually kill) and waitCh, which receives the cmd.Wait() result
// exactly once when ssh exits. On failure the ssh process has already
// been killed and waited.
func (c *SSHConnector) tryServe(ctx context.Context, p Profile, binary string, port int) (string, *exec.Cmd, <-chan error, error) {
	args := buildSSHBaseArgs(p)
	// Allocate a pseudo-TTY so SIGHUP propagates when the launcher exits.
	args = append(args, "-t")
	// Forward the Hub port to localhost so the local browser can reach it.
	args = append(args, "-L", fmt.Sprintf("127.0.0.1:%d:127.0.0.1:%d", port, port))
	args = append(args, sshTarget(p))
	// Remote command: launch many-ai-cli serve.
	// MANY_AI_CLI_HOST_LABEL: プロファイルの接続先 host をバッジ表示用ラベルとして
	// Hub に渡す（コンテナ内実行等で Hub 自身が外側の IP を検出できないため）。
	remoteCmd := fmt.Sprintf("exec env MANY_AI_CLI_HOST_LABEL=%s %s serve --port %d",
		ShellQuote(p.Host), ShellQuote(binary), port)
	if p.CWD != "" {
		remoteCmd = fmt.Sprintf("cd %s && %s", ShellQuote(p.CWD), remoteCmd)
	}
	args = append(args, "--", "bash", "-ilc", remoteCmd)

	cmd := exec.CommandContext(ctx, sshExe, args...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "", nil, nil, fmt.Errorf("stdout pipe: %w", err)
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return "", nil, nil, fmt.Errorf("stderr pipe: %w", err)
	}
	if err := cmd.Start(); err != nil {
		return "", nil, nil, fmt.Errorf("start %s: %w", sshExe, err)
	}

	urlFound := make(chan string, 1)
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		ScanForURL(stdout, c.stdoutWriter(), urlFound)
	}()
	go func() {
		defer wg.Done()
		ScanForURL(stderr, c.stderrWriter(), urlFound)
	}()

	// waitCh receives the error from cmd.Wait() after stdout/stderr drain.
	waitCh := make(chan error, 1)
	go func() {
		wg.Wait()
		waitCh <- cmd.Wait()
	}()

	select {
	case url := <-urlFound:
		// Got a URL; verify it matches the expected port.
		if !strings.Contains(url, fmt.Sprintf("127.0.0.1:%d/", port)) {
			// Port mismatch: remote serve auto-selected a different port.
			// Kill and let the caller retry with the next port candidate.
			_ = cmd.Process.Kill()
			<-waitCh
			return "", nil, nil, fmt.Errorf("port mismatch: expected %d in %s", port, url)
		}
		// Success: return the live cmd and its wait channel to the caller.
		return url, cmd, waitCh, nil

	case err := <-waitCh:
		if err != nil {
			return "", nil, nil, fmt.Errorf("%s exited: %w", sshExe, err)
		}
		return "", nil, nil, fmt.Errorf("%s exited before Hub URL was detected", sshExe)

	case <-ctx.Done():
		_ = cmd.Process.Kill()
		<-waitCh
		return "", nil, nil, ctx.Err()
	}
}

// cleanupSSHOrphans kills the remote many-ai-cli serve process matching port.
// Best-effort: pkill exits 1 when nothing matched. A 5s timeout guards against
// hung ssh or shutdown-in-progress.
func cleanupSSHOrphans(p Profile, binary string, port int) {
	// pkill -f のパターンはリモートのログインシェルを経由する（ssh は remote command を
	// 空白連結し、シェルが再パースする）。QuoteMeta で binary 内の正規表現メタ文字を
	// リテラル化し pkill の ERE が広がるのを防ぎ、ShellQuote でパターン全体を 1 つの
	// シェルトークンにして、シェルメタ文字を含む binary が別コマンドを注入できないようにする。
	pattern := regexp.QuoteMeta(binary) + fmt.Sprintf(" serve --port %d", port)
	args := buildSSHBaseArgs(p)
	args = append(args, sshTarget(p))
	args = append(args, "--", "pkill", "-f", ShellQuote(pattern))

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, sshExe, args...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	_ = cmd.Run()
}

// --------------------------------------------------------------------------
// tunnel mode
// --------------------------------------------------------------------------

const (
	tunnelTokenTimeout = 5 * time.Second
	tunnelPollInterval = 500 * time.Millisecond
	tunnelPollMaxTries = 20
)

func (c *SSHConnector) runTunnel(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) {
	port := p.HubPort // validated: must be ≥1

	// Step 1: establish the SSH tunnel (-N, no remote command).
	tunnelArgs := buildSSHBaseArgs(p)
	tunnelArgs = append(tunnelArgs, "-N")
	tunnelArgs = append(tunnelArgs, "-L", fmt.Sprintf("127.0.0.1:%d:127.0.0.1:%d", port, port))
	tunnelArgs = append(tunnelArgs, sshTarget(p))

	tunnel := exec.CommandContext(ctx, sshExe, tunnelArgs...)
	tunnel.SysProcAttr = noWindowSysProcAttr()
	tunnel.Stderr = c.stderrWriter()
	if err := tunnel.Start(); err != nil {
		sendErr(ctx, errCh, fmt.Errorf("start ssh tunnel: %w", err))
		close(urlCh)
		close(errCh)
		return
	}
	// tunnel.Wait() の結果を一度だけ受け、その後 close する。受信済みでも
	// defer 側のドレインが closed channel からゼロ値を読むだけで詰まらない。
	waitCh := make(chan error, 1)
	go func() {
		waitCh <- tunnel.Wait()
		close(waitCh)
	}()
	// Kill tunnel on exit.
	defer func() {
		_ = tunnel.Process.Kill()
		<-waitCh
	}()

	// Step 2: obtain the token via token_command.
	token, err := fetchToken(ctx, p)
	if err != nil {
		sendErr(ctx, errCh, err)
		close(urlCh)
		close(errCh)
		return
	}

	// Step 3: poll /api/info?token=<token> until the Hub responds 200.
	apiURL := fmt.Sprintf("http://127.0.0.1:%d/api/info?token=%s", port, url.QueryEscape(token))
	if err := pollUntilReady(ctx, apiURL); err != nil {
		sendErr(ctx, errCh, fmt.Errorf("hub not ready: %w", err))
		close(urlCh)
		close(errCh)
		return
	}

	// Step 3.5: 接続元情報（SSH 経由・接続先 host）を Hub に登録する。
	// URL クエリヒントを持たないクライアント（PWA・別タブ等）でも
	// /api/info 経由で正しいバッジが出るようにする。失敗しても接続は続行。
	postNetHint(ctx, port, token, p.Host)

	// Step 4: send the Hub URL to the caller for browser open.
	hubURL := buildTunnelHubURL(port, token, p.Host)
	select {
	case urlCh <- hubURL:
	case <-ctx.Done():
		close(urlCh)
		close(errCh)
		return
	}
	close(urlCh)

	// Step 5: keep the tunnel alive until it exits or ctx is cancelled.
	// ssh の終了（リモート Hub 停止・回線断）時は errCh を close して
	// launcher 本体に「接続終了」を伝える（コンソール窓の残骸防止）。
	select {
	case <-waitCh:
	case <-ctx.Done():
	}
	close(errCh)
}

// buildTunnelHubURL builds the browser-facing Hub URL for tunnel mode.
// via=ssh / host_label: tunnel モードでは既存 Hub に後から環境変数を渡せない
// （MANY_AI_CLI_HOST_LABEL は serve モード起動時のみ注入可能）ため、
// バッジ表示用の SSH 判定とホスト名を URL クエリで Hub UI へ伝える。
func buildTunnelHubURL(port int, token, host string) string {
	return fmt.Sprintf("http://127.0.0.1:%d/?token=%s&via=ssh&host_label=%s&env_kind=remote-tunnel",
		port, url.QueryEscape(token), url.QueryEscape(host))
}

// fetchToken runs token_command on the remote host via a short-lived SSH
// connection and returns the trimmed stdout.
func fetchToken(ctx context.Context, p Profile) (string, error) {
	timeoutCtx, cancel := context.WithTimeout(ctx, tunnelTokenTimeout)
	defer cancel()

	args := buildSSHBaseArgs(p)
	args = append(args, sshTarget(p))
	args = append(args, "--", p.TokenCommand)

	cmd := exec.CommandContext(timeoutCtx, sshExe, args...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("token_command %q: %w", p.TokenCommand, err)
	}
	token := strings.TrimSpace(string(out))
	if token == "" {
		return "", fmt.Errorf("token_command %q returned empty output", p.TokenCommand)
	}
	if _, err := normalizeHubToken(token); err != nil {
		return "", fmt.Errorf("token_command %q returned invalid token: %w", p.TokenCommand, err)
	}
	return token, nil
}

func normalizeHubToken(token string) (string, error) {
	if token == "" {
		return "", fmt.Errorf("empty token")
	}
	for _, r := range token {
		if unicode.IsSpace(r) || r < 0x20 || r == 0x7f {
			return "", fmt.Errorf("token must not contain whitespace or control characters")
		}
	}
	return token, nil
}

// pollUntilReady polls apiURL until it returns HTTP 200 or ctx is cancelled.
func pollUntilReady(ctx context.Context, apiURL string) error {
	client := &http.Client{Timeout: tunnelPollInterval}
	for i := 0; i < tunnelPollMaxTries; i++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, apiURL, nil)
		if err != nil {
			return err
		}
		resp, err := client.Do(req)
		if err == nil {
			_ = resp.Body.Close()
			if resp.StatusCode == http.StatusOK {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(tunnelPollInterval):
		}
	}
	return fmt.Errorf("timed out after %d polls", tunnelPollMaxTries)
}

// postNetHint は Hub の /api/net-hint に接続元情報（SSH 経由・接続先 host）を
// 登録する。tunnel モードでは既起動の Hub に MANY_AI_CLI_HOST_LABEL を注入
// できないため、API 経由でサーバ側に保持させる。best-effort（失敗は無視）。
func postNetHint(ctx context.Context, port int, token, host string) {
	payload, err := json.Marshal(map[string]any{"ssh": true, "host_label": host, "env_kind": "remote-tunnel"})
	if err != nil {
		return
	}
	apiURL := fmt.Sprintf("http://127.0.0.1:%d/api/net-hint?token=%s", port, url.QueryEscape(token))
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, apiURL, bytes.NewReader(payload))
	if err != nil {
		return
	}
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: tunnelTokenTimeout}
	if resp, err := client.Do(req); err == nil {
		_ = resp.Body.Close()
	}
}

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

// buildSSHBaseArgs returns the common ssh flags shared by all SSH profiles.
// It does NOT include -t / -N / -L / target / remote-command — callers add
// those depending on serve vs tunnel mode.
func buildSSHBaseArgs(p Profile) []string {
	args := []string{
		"-o", "BatchMode=yes",
		"-o", "ExitOnForwardFailure=yes",
		"-o", "ServerAliveInterval=30",
	}
	if p.SSHPort > 0 {
		args = append(args, "-p", fmt.Sprintf("%d", p.SSHPort))
	}
	if p.IdentityFile != "" {
		args = append(args, "-i", p.IdentityFile)
	}
	return args
}

// sshTarget returns "[user@]host" from the profile.
func sshTarget(p Profile) string {
	if p.User != "" {
		return p.User + "@" + p.Host
	}
	return p.Host
}

// sendErr sends err to errCh unless ctx is already done.
func sendErr(ctx context.Context, errCh chan<- error, err error) {
	select {
	case errCh <- err:
	case <-ctx.Done():
	}
}

// Ensure SSHConnector satisfies the Connector interface at compile time.
var _ Connector = (*SSHConnector)(nil)

// NewSSHConnector returns an SSHConnector as a Connector interface value.
func NewSSHConnector() Connector {
	return &SSHConnector{}
}

// OpenBrowserOnce opens the browser for url and reports the error (if any)
// to stderr. Intended as a convenience wrapper around OpenBrowser for
// connectors that receive a url from urlCh.
func OpenBrowserOnce(url string) {
	if err := OpenBrowser(url); err != nil {
		fmt.Fprintf(os.Stderr, "many-ai-cli-launcher: failed to open browser for %s: %v\n", url, err)
	}
}

// DrainURL reads exactly one URL from urlCh and opens the browser, or returns
// when ctx is done. Call this after Start to wire up browser opening.
func DrainURL(ctx context.Context, urlCh <-chan string) {
	select {
	case url, ok := <-urlCh:
		if ok && url != "" {
			OpenBrowserOnce(url)
		}
	case <-ctx.Done():
	}
}

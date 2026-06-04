//go:build windows

package launcher

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"
)

// SSHConnector implements Connector for SSH profiles (serve and tunnel modes).
type SSHConnector struct{}

// Start launches the SSH connection described by p.
// For serve mode it starts `ssh.exe -t -L …` which runs `any-ai-cli serve`
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
		binary = "any-ai-cli"
	}

	basePort := p.HubPort
	if basePort == 0 {
		basePort = PickPort()
	}

	var lastErr error
	for attempt := 0; attempt < sshMaxRetries; attempt++ {
		port := basePort + attempt*sshPortStep

		url, cmd, err := c.tryServe(ctx, p, binary, port)
		if err != nil {
			lastErr = err
			if ctx.Err() != nil {
				break
			}
			continue
		}

		// URL obtained successfully. Send it, then keep the ssh process alive
		// until ctx is cancelled; clean up remote serve on exit.
		select {
		case urlCh <- url:
		case <-ctx.Done():
		}
		close(urlCh)
		close(errCh)

		// Wait for context cancellation, then kill and clean up.
		<-ctx.Done()
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		cleanupSSHOrphans(p, binary, port)
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

// tryServe starts ssh.exe and blocks until the Hub URL is detected or an error
// occurs. On success it returns the URL and the live *exec.Cmd (which the
// caller must eventually kill). On failure the ssh process has already been
// killed and waited.
func (c *SSHConnector) tryServe(ctx context.Context, p Profile, binary string, port int) (string, *exec.Cmd, error) {
	args := buildSSHBaseArgs(p)
	// Allocate a pseudo-TTY so SIGHUP propagates when the launcher exits.
	args = append(args, "-t")
	// Forward the Hub port to localhost so the Windows browser can reach it.
	args = append(args, "-L", fmt.Sprintf("127.0.0.1:%d:127.0.0.1:%d", port, port))
	args = append(args, sshTarget(p))
	// Remote command: launch any-ai-cli serve.
	remoteCmd := fmt.Sprintf("exec %s serve --port %d", ShellQuote(binary), port)
	if p.CWD != "" {
		remoteCmd = fmt.Sprintf("cd %s && %s", ShellQuote(p.CWD), remoteCmd)
	}
	args = append(args, "--", "bash", "-ilc", remoteCmd)

	cmd := exec.CommandContext(ctx, "ssh.exe", args...)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "", nil, fmt.Errorf("stdout pipe: %w", err)
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return "", nil, fmt.Errorf("stderr pipe: %w", err)
	}
	if err := cmd.Start(); err != nil {
		return "", nil, fmt.Errorf("start ssh.exe: %w", err)
	}

	urlFound := make(chan string, 1)
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		ScanForURL(stdout, os.Stdout, urlFound)
	}()
	go func() {
		defer wg.Done()
		ScanForURL(stderr, os.Stderr, urlFound)
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
			return "", nil, fmt.Errorf("port mismatch: expected %d in %s", port, url)
		}
		// Success: return the live cmd to the caller.
		return url, cmd, nil

	case err := <-waitCh:
		if err != nil {
			return "", nil, fmt.Errorf("ssh.exe exited: %w", err)
		}
		return "", nil, fmt.Errorf("ssh.exe exited before Hub URL was detected")

	case <-ctx.Done():
		_ = cmd.Process.Kill()
		<-waitCh
		return "", nil, ctx.Err()
	}
}

// cleanupSSHOrphans kills the remote any-ai-cli serve process matching port.
// Best-effort: pkill exits 1 when nothing matched. A 5s timeout guards against
// hung ssh or shutdown-in-progress.
func cleanupSSHOrphans(p Profile, binary string, port int) {
	pattern := fmt.Sprintf("%s serve --port %d", binary, port)
	args := buildSSHBaseArgs(p)
	args = append(args, sshTarget(p))
	args = append(args, "--", "pkill", "-f", pattern)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = exec.CommandContext(ctx, "ssh.exe", args...).Run()
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

	tunnel := exec.CommandContext(ctx, "ssh.exe", tunnelArgs...)
	tunnel.Stderr = os.Stderr
	if err := tunnel.Start(); err != nil {
		sendErr(ctx, errCh, fmt.Errorf("start ssh tunnel: %w", err))
		close(urlCh)
		close(errCh)
		return
	}
	// Kill tunnel on exit.
	defer func() {
		_ = tunnel.Process.Kill()
		_ = tunnel.Wait()
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
	apiURL := fmt.Sprintf("http://127.0.0.1:%d/api/info?token=%s", port, token)
	if err := pollUntilReady(ctx, apiURL); err != nil {
		sendErr(ctx, errCh, fmt.Errorf("hub not ready: %w", err))
		close(urlCh)
		close(errCh)
		return
	}

	// Step 4: send the Hub URL to the caller for browser open.
	hubURL := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", port, token)
	select {
	case urlCh <- hubURL:
	case <-ctx.Done():
		close(urlCh)
		close(errCh)
		return
	}
	close(urlCh)
	close(errCh)

	// Step 5: keep the tunnel alive until ctx is cancelled.
	<-ctx.Done()
}

// fetchToken runs token_command on the remote host via a short-lived SSH
// connection and returns the trimmed stdout.
func fetchToken(ctx context.Context, p Profile) (string, error) {
	timeoutCtx, cancel := context.WithTimeout(ctx, tunnelTokenTimeout)
	defer cancel()

	args := buildSSHBaseArgs(p)
	args = append(args, sshTarget(p))
	args = append(args, "--", p.TokenCommand)

	out, err := exec.CommandContext(timeoutCtx, "ssh.exe", args...).Output()
	if err != nil {
		return "", fmt.Errorf("token_command %q: %w", p.TokenCommand, err)
	}
	token := strings.TrimSpace(string(out))
	if token == "" {
		return "", fmt.Errorf("token_command %q returned empty output", p.TokenCommand)
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

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

// buildSSHBaseArgs returns the common ssh.exe flags shared by all SSH profiles.
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
		fmt.Fprintf(os.Stderr, "any-ai-cli-launcher: failed to open browser for %s: %v\n", url, err)
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

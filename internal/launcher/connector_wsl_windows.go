//go:build windows

package launcher

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sync"
	"time"
)

// WSLConnector implements Connector for WSL profiles.
//
// out mirrors SSHConnector.out: nil keeps the launcher-exe behaviour
// (os.Stdout / os.Stderr); the Hub sets io.Discard via setQuiet for quiet,
// windowless hosting.
type WSLConnector struct {
	out io.Writer
}

func (c *WSLConnector) stdoutWriter() io.Writer {
	if c.out != nil {
		return c.out
	}
	return os.Stdout
}

func (c *WSLConnector) stderrWriter() io.Writer {
	if c.out != nil {
		return c.out
	}
	return os.Stderr
}

// setQuiet implements the quietable interface used by ConnectorForQuiet.
func (c *WSLConnector) setQuiet(w io.Writer) { c.out = w }

// Start launches `wsl.exe -- many-ai-cli serve` in the target distribution,
// watches stdout/stderr for the Hub URL, then opens the default browser.
func (c *WSLConnector) Start(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) error {
	if p.Type != ProfileTypeWSL {
		return fmt.Errorf("WSLConnector requires a wsl profile, got %q", p.Type)
	}
	go c.run(ctx, p, urlCh, errCh)
	return nil
}

func (c *WSLConnector) run(ctx context.Context, p Profile, urlCh chan<- string, errCh chan<- error) {
	binary := p.Binary
	if binary == "" {
		binary = "many-ai-cli"
	}

	port := p.HubPort
	if port == 0 {
		port = PickPort()
	}

	defer cleanupWSLOrphansConnector(p.Distro, port)

	cwd := p.CWD
	if cwd == "" {
		cwd = "~"
	}

	var wslArgs []string
	if p.Distro != "" {
		wslArgs = append(wslArgs, "-d", p.Distro)
	}
	if cwd != "" {
		wslArgs = append(wslArgs, "--cd", cwd)
	}
	// Invoke through `bash -ilc` (login + interactive). Login alone is not
	// enough on a default Ubuntu setup: ~/.bashrc returns early for
	// non-interactive shells, skipping pnpm/nvm/cargo PATH setup at its
	// tail. The symptom is that `which codex` then resolves to the
	// Windows-side pnpm shim surfaced via WSL interop, which fails inside
	// WSL because `node` is not installed there. Adding -i bypasses that
	// early-return guard so the user's real PATH is in effect.
	// MANY_AI_CLI_WSL_LAUNCHER marks "the user is reaching the WSL Hub from
	// Windows via this launcher", so the Linux-side serve can default log_dir
	// to the Windows %USERPROFILE% (so the Hub UI's open-folder buttons land
	// in plain C:\Users\... instead of \\wsl$\... UNC). A bare `many-ai-cli
	// serve` inside WSL (without this launcher) stays purely Linux-side.
	shellCmd := fmt.Sprintf("export MANY_AI_CLI_WSL_LAUNCHER=1; exec %s serve --port %d", ShellQuote(binary), port)
	wslArgs = append(wslArgs, "--", "bash", "-ilc", shellCmd)

	cmd := exec.CommandContext(ctx, "wsl.exe", wslArgs...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		sendErr(ctx, errCh, fmt.Errorf("stdout pipe: %w", err))
		close(urlCh)
		close(errCh)
		return
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		sendErr(ctx, errCh, fmt.Errorf("stderr pipe: %w", err))
		close(urlCh)
		close(errCh)
		return
	}
	if err := cmd.Start(); err != nil {
		sendErr(ctx, errCh, fmt.Errorf("start wsl.exe: %w", err))
		close(urlCh)
		close(errCh)
		return
	}

	foundCh := make(chan string, 1)
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		ScanForURL(stdout, c.stdoutWriter(), foundCh)
	}()
	go func() {
		defer wg.Done()
		ScanForURL(stderr, c.stderrWriter(), foundCh)
	}()

	go func() {
		select {
		case url := <-foundCh:
			select {
			case urlCh <- url:
			case <-ctx.Done():
			}
		case <-ctx.Done():
		}
	}()

	go func() {
		<-ctx.Done()
		_ = cmd.Process.Kill()
	}()

	waitErr := cmd.Wait()
	wg.Wait()

	if waitErr != nil && ctx.Err() == nil {
		sendErr(ctx, errCh, fmt.Errorf("wsl.exe exited: %w", waitErr))
	}
	// wsl.exe の正常終了（= WSL 側 serve の停止。Web UI の「Web のみ停止」
	// を含む）でもここに到達する。errCh の close は「接続終了」の合図で、
	// launcher 本体はこれを受けてプロセスを終了する（コンソール窓の残骸防止）。
	close(urlCh)
	close(errCh)
}

// cleanupWSLOrphansConnector terminates the WSL-side many-ai-cli serve process
// this launcher run started. WSL2 interop does not propagate SIGHUP/SIGTERM
// from the Windows side, so the Linux serve survives wsl.exe getting killed
// and continues to hold the Hub port. We match on the exact --port the
// launcher used so we don't kill unrelated serve sessions on other ports.
//
// Best-effort: pkill exits 1 when nothing matched (the typical success path
// when the serve already exited cleanly), so we ignore the return entirely.
// A 5s timeout guards against wsl.exe being hung or shutdown-in-progress —
// we'd rather exit the launcher than block forever on cleanup.
func cleanupWSLOrphansConnector(distro string, port int) {
	var args []string
	if distro != "" {
		args = append(args, "-d", distro)
	}
	pattern := fmt.Sprintf("many-ai-cli serve --port %d", port)
	args = append(args, "--", "pkill", "-f", pattern)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "wsl.exe", args...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	_ = cmd.Run()
}

// Ensure WSLConnector satisfies the Connector interface at compile time.
var _ Connector = (*WSLConnector)(nil)

// NewWSLConnector returns a WSLConnector as a Connector interface value.
func NewWSLConnector() Connector {
	return &WSLConnector{}
}

// connectorForWSL returns a WSLConnector on Windows.
func connectorForWSL() (Connector, error) {
	return NewWSLConnector(), nil
}

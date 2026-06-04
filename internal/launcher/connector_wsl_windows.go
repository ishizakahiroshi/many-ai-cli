//go:build windows

package launcher

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"sync"
	"time"
)

// WSLConnector implements Connector for WSL profiles.
type WSLConnector struct{}

// Start launches `wsl.exe -- any-ai-cli serve` in the target distribution,
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
		binary = "any-ai-cli"
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
	// See cmd/any-ai-cli-wsl/main_windows.go for the rationale behind
	// bash -ilc and ANY_AI_CLI_WSL_LAUNCHER.
	shellCmd := fmt.Sprintf("export ANY_AI_CLI_WSL_LAUNCHER=1; exec %s serve --port %d", ShellQuote(binary), port)
	wslArgs = append(wslArgs, "--", "bash", "-ilc", shellCmd)

	cmd := exec.CommandContext(ctx, "wsl.exe", wslArgs...)
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
		ScanForURL(stdout, os.Stdout, foundCh)
	}()
	go func() {
		defer wg.Done()
		ScanForURL(stderr, os.Stderr, foundCh)
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
	close(urlCh)
	close(errCh)
}

// cleanupWSLOrphansConnector is the Connector-layer equivalent of the top-level
// cleanupWSLOrphans in cmd/any-ai-cli-wsl. Terminates the Linux-side serve
// that was started by this launcher run.
func cleanupWSLOrphansConnector(distro string, port int) {
	var args []string
	if distro != "" {
		args = append(args, "-d", distro)
	}
	pattern := fmt.Sprintf("any-ai-cli serve --port %d", port)
	args = append(args, "--", "pkill", "-f", pattern)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = exec.CommandContext(ctx, "wsl.exe", args...).Run()
}

// Ensure WSLConnector satisfies the Connector interface at compile time.
var _ Connector = (*WSLConnector)(nil)

// NewWSLConnector returns a WSLConnector as a Connector interface value.
func NewWSLConnector() Connector {
	return &WSLConnector{}
}

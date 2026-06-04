//go:build windows

// any-ai-cli-wsl is a tiny Windows launcher that does one thing: spawn
// `wsl.exe -- any-ai-cli serve` inside the user's WSL distribution, watch
// the child's output for the Hub URL, then open that URL in the default
// Windows browser. All real logic lives in the Linux binary inside WSL.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"any-ai-cli/internal/launcher"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	launcher.ConfigureConsoleUTF8()

	fs := flag.NewFlagSet("any-ai-cli-wsl", flag.ContinueOnError)
	distro := fs.String("distro", "", "WSL distribution name (defaults to wsl.exe's default distro)")
	binary := fs.String("binary", "any-ai-cli", "any-ai-cli binary name inside WSL (must be on PATH)")
	port := fs.Int("port", 0, "port for the WSL Hub (0 = auto-pick to avoid Windows-side collisions)")
	// Default to ~ (WSL HOME). Without this, wsl.exe inherits the Windows
	// caller's cwd (e.g. C:\...) and remaps it to /mnt/c/... — which makes
	// the Hub's "new session" default to a Win-side path, defeating the
	// point of running inside WSL. Pass --cwd "" to keep the inherited cwd.
	cwd := fs.String("cwd", "~", `working directory inside WSL (e.g. "~", "/home/user/projects"); empty to inherit caller cwd`)
	if err := fs.Parse(os.Args[1:]); err != nil {
		return err
	}

	chosenPort := *port
	if chosenPort == 0 {
		chosenPort = launcher.PickPort()
	}

	// WSL2 interop does not propagate SIGHUP from the Windows side, so when
	// wsl.exe dies (Ctrl+C / launcher exit / parent close) the Linux-side
	// any-ai-cli serve process keeps running with closed stdio pipes — an
	// orphan that holds the Hub port and prevents subsequent launcher runs
	// from binding. Pkill it on launcher exit, scoped to *this* launcher's
	// --port so concurrent serve sessions on other ports survive.
	defer cleanupWSLOrphans(*distro, chosenPort)

	var wslArgs []string
	if *distro != "" {
		wslArgs = append(wslArgs, "-d", *distro)
	}
	if *cwd != "" {
		wslArgs = append(wslArgs, "--cd", *cwd)
	}
	// Invoke through `bash -ilc` (login + interactive). Login alone is not
	// enough on a default Ubuntu setup: ~/.bashrc returns early for
	// non-interactive shells, skipping pnpm/nvm/cargo PATH setup at its
	// tail. The symptom is that `which codex` then resolves to the
	// Windows-side pnpm shim surfaced via WSL interop, which fails inside
	// WSL because `node` is not installed there. Adding -i bypasses that
	// early-return guard so the user's real PATH is in effect.
	// ANY_AI_CLI_WSL_LAUNCHER marks "the user is reaching the WSL Hub from
	// Windows via this launcher", so the Linux-side serve can default log_dir
	// to the Windows %USERPROFILE% (so the Hub UI's open-folder buttons land
	// in plain C:\Users\... instead of \\wsl$\... UNC). A bare `any-ai-cli
	// serve` inside WSL (without this launcher) stays purely Linux-side.
	shellCmd := fmt.Sprintf("export ANY_AI_CLI_WSL_LAUNCHER=1; exec %s serve --port %d", launcher.ShellQuote(*binary), chosenPort)
	wslArgs = append(wslArgs, "--", "bash", "-ilc", shellCmd)

	cmd := exec.Command("wsl.exe", wslArgs...)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return fmt.Errorf("stdout pipe: %w", err)
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return fmt.Errorf("stderr pipe: %w", err)
	}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start wsl.exe: %w", err)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	var (
		opened   sync.Once
		urlFound = make(chan string, 1)
		wg       sync.WaitGroup
	)
	openOnce := func(url string) {
		opened.Do(func() {
			if err := launcher.OpenBrowser(url); err != nil {
				fmt.Fprintf(os.Stderr, "any-ai-cli-wsl: failed to open browser for %s: %v\n", url, err)
			}
		})
	}

	scan := func(r io.Reader, w io.Writer) {
		defer wg.Done()
		launcher.ScanForURL(r, w, urlFound)
	}
	wg.Add(2)
	go scan(stdout, os.Stdout)
	go scan(stderr, os.Stderr)

	go func() {
		select {
		case url := <-urlFound:
			openOnce(url)
		case <-ctx.Done():
		}
	}()

	go func() {
		<-ctx.Done()
		_ = cmd.Process.Kill()
	}()

	waitErr := cmd.Wait()
	wg.Wait()
	if waitErr != nil {
		var exitErr *exec.ExitError
		if errors.As(waitErr, &exitErr) {
			return fmt.Errorf("wsl.exe exited: %w", exitErr)
		}
		return waitErr
	}
	return nil
}

// cleanupWSLOrphans terminates the WSL-side any-ai-cli serve process this
// launcher started. WSL2 interop does not propagate SIGHUP/SIGTERM from the
// Windows side, so the Linux serve survives wsl.exe getting killed and
// continues to hold the Hub port. We match on the exact --port the launcher
// used so we don't kill unrelated serve sessions running on other ports.
//
// Best-effort: pkill exits 1 when nothing matched (the typical success path
// when the serve already exited cleanly), so we ignore the return entirely.
// A 5s timeout guards against wsl.exe being hung or shutdown-in-progress —
// we'd rather exit the launcher than block forever on cleanup.
func cleanupWSLOrphans(distro string, port int) {
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

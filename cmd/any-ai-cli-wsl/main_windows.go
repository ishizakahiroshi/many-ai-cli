//go:build windows

// any-ai-cli-wsl is a tiny Windows launcher that does one thing: spawn
// `wsl.exe -- any-ai-cli serve` inside the user's WSL distribution, watch
// the child's output for the Hub URL, then open that URL in the default
// Windows browser. All real logic lives in the Linux binary inside WSL.
package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"os/signal"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

const (
	defaultHubPort = 47777
	portStep       = 100
	maxPortProbes  = 10
	probeTimeout   = 200 * time.Millisecond
)

// hubURLRe matches the Hub URL printed in the startup banner.
// Token is hex-only — anchoring to [0-9a-fA-F]+ avoids accidentally
// consuming trailing ANSI escapes or other non-whitespace junk that
// might appear on the same line in some terminals.
var hubURLRe = regexp.MustCompile(`http://127\.0\.0\.1:\d+/\?token=[0-9a-fA-F]+`)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
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
		chosenPort = pickPort()
	}

	var wslArgs []string
	if *distro != "" {
		wslArgs = append(wslArgs, "-d", *distro)
	}
	if *cwd != "" {
		wslArgs = append(wslArgs, "--cd", *cwd)
	}
	// Invoke through `bash -lc` so PATH resolution honors the user's login
	// shell setup. Without this, `wsl.exe -- any-ai-cli ...` runs with a
	// minimal PATH that does not include user-local install dirs like
	// ~/.local/bin or ~/bin, even though `which any-ai-cli` works in an
	// interactive shell.
	shellCmd := fmt.Sprintf("exec %s serve --port %d", shellQuote(*binary), chosenPort)
	wslArgs = append(wslArgs, "--", "bash", "-lc", shellCmd)

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
			if err := openBrowser(url); err != nil {
				fmt.Fprintf(os.Stderr, "any-ai-cli-wsl: failed to open browser for %s: %v\n", url, err)
			}
		})
	}

	scan := func(r io.Reader, w io.Writer) {
		defer wg.Done()
		s := bufio.NewScanner(r)
		s.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for s.Scan() {
			line := s.Text()
			fmt.Fprintln(w, line)
			if match := hubURLRe.FindString(line); match != "" {
				select {
				case urlFound <- match:
				default:
				}
			}
		}
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

// openBrowser launches the Windows default browser at url.
// Uses `cmd /c start "" <url>` which is the documented stable way to invoke
// the user's registered URL handler. The empty "" is start's window-title
// argument and prevents start from treating the URL itself as a title.
func openBrowser(url string) error {
	return exec.Command("cmd", "/c", "start", "", url).Start()
}

// shellQuote wraps s in single quotes for safe POSIX shell expansion via
// `bash -lc '...'`. Embedded single quotes are escaped using the classic
// `'\''` close/quote/reopen idiom.
func shellQuote(s string) string {
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

// pickPort returns the first port (defaultHubPort, +portStep, +2*portStep, ...)
// that has no Windows-side listener responding within probeTimeout.
// This avoids the WSL-Hub-on-47777 case getting shadowed by a Windows-native
// Hub already bound to 47777 (Windows side wins for localhost forwarding).
// If all probes find something listening, fall back to defaultHubPort and let
// the WSL-side serve port-scan as usual.
func pickPort() int {
	for i := 0; i < maxPortProbes; i++ {
		port := defaultHubPort + i*portStep
		if !windowsPortInUse(port) {
			return port
		}
	}
	return defaultHubPort
}

func windowsPortInUse(port int) bool {
	conn, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", port), probeTimeout)
	if err != nil {
		return false
	}
	_ = conn.Close()
	return true
}

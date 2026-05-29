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
	"strings"
	"sync"
	"syscall"
	"time"
	"unsafe"
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

var (
	kernel32            = syscall.NewLazyDLL("kernel32.dll")
	procSetConsoleCP    = kernel32.NewProc("SetConsoleCP")
	procSetConsoleOutCP = kernel32.NewProc("SetConsoleOutputCP")
	procGetStdHandle    = kernel32.NewProc("GetStdHandle")
	procGetConsoleMode  = kernel32.NewProc("GetConsoleMode")
	procSetConsoleMode  = kernel32.NewProc("SetConsoleMode")
)

const (
	// Win32 STD_OUTPUT_HANDLE / STD_ERROR_HANDLE — DWORD-cast of -11 / -12.
	stdOutputHandle = uint32(0xFFFFFFF5)
	stdErrorHandle  = uint32(0xFFFFFFF4)

	// SetConsoleMode flags. We make sure PROCESSED_OUTPUT (so "\n" expands to
	// CRLF) and VIRTUAL_TERMINAL_PROCESSING (so ANSI color escapes work) are
	// both on; their states are independent and both matter for the banner.
	enableProcessedOutput           = 0x0001
	enableWrapAtEOLOutput           = 0x0002
	enableVirtualTerminalProcessing = 0x0004
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	configureConsoleUTF8()

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
	shellCmd := fmt.Sprintf("export ANY_AI_CLI_WSL_LAUNCHER=1; exec %s serve --port %d", shellQuote(*binary), chosenPort)
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
			// The Linux-side serve writes LF-only line breaks (Go's
			// fmt.Println behavior). Belt-and-braces with the SetConsoleMode
			// call above: emit explicit CRLF here so the banner renders
			// correctly even if some other process (or a future Windows
			// build) re-disables PROCESSED_OUTPUT on this console.
			_, _ = io.WriteString(w, line+"\r\n")
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

func configureConsoleUTF8() {
	const cpUTF8 = 65001
	_, _, _ = procSetConsoleCP.Call(cpUTF8)
	_, _, _ = procSetConsoleOutCP.Call(cpUTF8)
	ensureConsoleOutputMode(stdOutputHandle)
	ensureConsoleOutputMode(stdErrorHandle)
}

// ensureConsoleOutputMode forces PROCESSED_OUTPUT and VT_PROCESSING on for the
// given std handle. Without PROCESSED_OUTPUT the Hub banner's "\n" line breaks
// only emit LF; cursor moves down without returning to column 0, and the next
// line is rendered starting at the previous line's right edge (the ASCII art
// "slides diagonally" and "Claude Code / Codex wrapper" stacks onto the logo's
// last row). Without VT_PROCESSING the ANSI color escapes in the banner are
// printed as raw text, blowing up apparent line width. Both modes are
// independent — we OR them in rather than overwriting, so any existing flags
// (line wrap, etc.) survive.
func ensureConsoleOutputMode(stdHandleID uint32) {
	h, _, _ := procGetStdHandle.Call(uintptr(int32(stdHandleID)))
	if h == 0 || h == uintptr(^uintptr(0)) {
		return
	}
	var mode uint32
	r, _, _ := procGetConsoleMode.Call(h, uintptr(unsafe.Pointer(&mode)))
	if r == 0 {
		return
	}
	newMode := mode | enableProcessedOutput | enableVirtualTerminalProcessing | enableWrapAtEOLOutput
	if newMode == mode {
		return
	}
	_, _, _ = procSetConsoleMode.Call(h, uintptr(newMode))
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

// openBrowser launches the Windows default browser at url.
// Uses `cmd /c start "" <url>` which is the documented stable way to invoke
// the user's registered URL handler. The empty "" is start's window-title
// argument and prevents start from treating the URL itself as a title.
func openBrowser(url string) error {
	return exec.Command("cmd", "/c", "start", "", url).Start()
}

// shellQuote wraps s in single quotes for safe POSIX shell expansion via
// `bash -lc '...'`. Embedded single quotes use the POSIX
// close/quote/reopen idiom.
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
		return !isConnectionRefused(err)
	}
	_ = conn.Close()
	return true
}

func isConnectionRefused(err error) bool {
	if err == nil {
		return false
	}
	msg := strings.ToLower(err.Error())
	return strings.Contains(msg, "connection refused") ||
		strings.Contains(msg, "actively refused") ||
		strings.Contains(msg, "no connection could be made")
}

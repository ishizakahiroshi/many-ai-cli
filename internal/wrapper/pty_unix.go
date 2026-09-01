//go:build !windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"
	"sync"
	"syscall"
	"time"

	"github.com/creack/pty"
)

type ptyProcess struct {
	f         *os.File
	cmd       *exec.Cmd
	closeOnce sync.Once
	waitOnce  sync.Once
	waitDone  chan struct{}
	waitErr   error
}

func (p *ptyProcess) Read(b []byte) (int, error)  { return p.f.Read(b) }
func (p *ptyProcess) Write(b []byte) (int, error) { return p.f.Write(b) }

// Close is idempotent: the first call asks the child to exit, closes the PTY
// master, then escalates to SIGKILL only if Wait does not complete within the
// grace period. Subsequent calls are no-ops.
func (p *ptyProcess) Close() error {
	var err error
	p.closeOnce.Do(func() {
		if p.cmd != nil && p.cmd.Process != nil {
			// Try graceful termination first; ignore errors (process may have
			// already exited).
			_ = p.cmd.Process.Signal(syscall.SIGTERM)
		}
		err = p.f.Close()

		done := make(chan struct{})
		go func() {
			_ = p.Wait()
			close(done)
		}()
		select {
		case <-done:
		case <-time.After(processCloseGrace):
			if p.cmd != nil && p.cmd.Process != nil {
				_ = p.cmd.Process.Kill()
			}
		}
	})
	return err
}

// Wait waits for the child process to exit. Kill errors (process already gone)
// are expected and can be safely ignored by callers.
func (p *ptyProcess) Wait() error {
	p.waitOnce.Do(func() {
		defer close(p.waitDone)
		if p.cmd != nil {
			p.waitErr = p.cmd.Wait()
		}
	})
	<-p.waitDone
	return p.waitErr
}

// exitSignalInfo reports whether exitErr represents a process terminated by
// a signal (e.g. OOM kill, SIGTERM/SIGKILL) and, if so, the signal's name.
// Used by classifyExit (wrapper.go) to distinguish signal-killed children
// from an ordinary non-zero exit in session_end logging.
func exitSignalInfo(exitErr *exec.ExitError) (bool, string) {
	ws, ok := exitErr.Sys().(syscall.WaitStatus)
	if !ok || !ws.Signaled() {
		return false, ""
	}
	return true, ws.Signal().String()
}

func (p *ptyProcess) Resize(cols, rows uint16) error {
	return pty.Setsize(p.f, &pty.Winsize{Rows: rows, Cols: cols})
}

func startProcess(provider string, customArgv []string, args []string, cwd string, cols, rows int, extraEnv []string) (processSession, error) {
	cmdName, cmdArgs := resolveCmd(provider, customArgv, args)
	cmd := exec.Command(cmdName, cmdArgs...)
	cmd.Dir = cwd
	// Hub のペインは xterm.js なので色を出せる。TERM / COLORTERM を上書きするのと同じ
	// 理由で FORCE_COLOR / CLICOLOR_FORCE も立てる。Hub を NO_COLOR=1 や
	// FORCE_COLOR=0 の環境（AI エージェントのハーネス配下など）から起こすと、TERM を
	// 上書きしても子 CLI が色を落とすため（実測: Linux の claude セッションは SGR 0 個、
	// 同日の Windows は色指定 15132 個）。os/exec は同じキーの最後の値を採用するので、
	// 継承した値はここで上書きされる。FORCE_COLOR=3 は 24bit、CLICOLOR_FORCE は
	// Rust 系 CLI 向け。継承した NO_COLOR は envWithoutColorSuppressors で落とす
	// （FORCE_COLOR を先に見ない provider では NO_COLOR が勝つため。実測は
	// docs/local/pending_wrap-inherits-no-color-from-hub-env.md）。
	cmd.Env = append(envWithoutColorSuppressors(os.Environ()), "TERM=xterm-256color", "COLORTERM=truecolor", "FORCE_COLOR=3", "CLICOLOR_FORCE=1", "MANY_AI_CLI=1")
	cmd.Env = append(cmd.Env, extraEnv...)
	var (
		f   *os.File
		err error
	)
	if cols > 0 && rows > 0 {
		f, err = pty.StartWithSize(cmd, &pty.Winsize{Cols: uint16(cols), Rows: uint16(rows)})
	} else {
		f, err = pty.Start(cmd)
	}
	if err != nil {
		return nil, fmt.Errorf("pty start %s: %w", provider, err)
	}
	return &ptyProcess{f: f, cmd: cmd, waitDone: make(chan struct{})}, nil
}

// resolveCmd picks the executable and final argv for this session.
// customArgv (non-nil only for a config.yaml custom_providers entry — see
// wrapper.customProviderFor) takes priority: its first element is the
// executable to resolve, the rest is prepended to args. Custom providers
// never reach the shell/copilot special cases below, and never fall into the
// bare exec.LookPath(provider) fallback either — provider there is a
// config.yaml id, not necessarily a real binary name.
func resolveCmd(provider string, customArgv []string, args []string) (string, []string) {
	if len(customArgv) > 0 {
		combined := append(append([]string{}, customArgv[1:]...), args...)
		if path, err := exec.LookPath(customArgv[0]); err == nil {
			return path, combined
		}
		return customArgv[0], combined
	}
	if provider == "shell" {
		return resolveDefaultShell(), args
	}
	if provider == "copilot" {
		if path, err := exec.LookPath("copilot"); err == nil {
			return path, args
		}
		if path, err := exec.LookPath("gh"); err == nil {
			return path, copilotViaGhArgs(args)
		}
		return provider, args
	}
	if path, err := exec.LookPath(provider); err == nil {
		return path, args
	}
	return provider, args
}

// resolveDefaultShell returns the path to the default interactive shell on
// Unix. Preference order: $SHELL env → bash → sh.
func resolveDefaultShell() string {
	if sh := os.Getenv("SHELL"); sh != "" {
		if _, err := os.Stat(sh); err == nil { // #nosec G703 -- sh は $SHELL 由来の既定シェルパスの存在確認で外部入力ではない
			return sh
		}
	}
	for _, name := range []string{"bash", "sh"} {
		if p, err := exec.LookPath(name); err == nil {
			return p
		}
	}
	return "/bin/sh"
}

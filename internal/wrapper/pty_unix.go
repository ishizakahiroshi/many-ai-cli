//go:build !windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"
	"sync"
	"syscall"

	"github.com/creack/pty"
)

type ptyProcess struct {
	f         *os.File
	cmd       *exec.Cmd
	closeOnce sync.Once
}

func (p *ptyProcess) Read(b []byte) (int, error)  { return p.f.Read(b) }
func (p *ptyProcess) Write(b []byte) (int, error) { return p.f.Write(b) }

// Close is idempotent: the first call signals the child process (SIGTERM then
// SIGKILL) and closes the PTY master file descriptor. Subsequent calls are
// no-ops. This prevents double-close panics when multiple goroutines (reconnect
// supervisor, PTY output loop) both attempt shutdown.
func (p *ptyProcess) Close() error {
	var err error
	p.closeOnce.Do(func() {
		if p.cmd != nil && p.cmd.Process != nil {
			// Try graceful termination first; ignore errors (process may have
			// already exited).
			_ = p.cmd.Process.Signal(syscall.SIGTERM)
		}
		err = p.f.Close()
		if p.cmd != nil && p.cmd.Process != nil {
			_ = p.cmd.Process.Kill()
		}
	})
	return err
}

// Wait waits for the child process to exit. Kill errors (process already gone)
// are expected and can be safely ignored by callers.
func (p *ptyProcess) Wait() error { return p.cmd.Wait() }

func (p *ptyProcess) Resize(cols, rows uint16) error {
	return pty.Setsize(p.f, &pty.Winsize{Rows: rows, Cols: cols})
}

func startProcess(provider string, args []string, cwd string, cols, rows int) (processSession, error) {
	cmd := exec.Command(provider, args...)
	cmd.Dir = cwd
	cmd.Env = append(os.Environ(), "TERM=xterm-256color", "COLORTERM=truecolor", "ANY_AI_CLI=1")
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
	return &ptyProcess{f: f, cmd: cmd}, nil
}

//go:build !windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"

	"github.com/creack/pty"
)

type ptyProcess struct {
	f   *os.File
	cmd *exec.Cmd
}

func (p *ptyProcess) Read(b []byte) (int, error)  { return p.f.Read(b) }
func (p *ptyProcess) Write(b []byte) (int, error) { return p.f.Write(b) }
func (p *ptyProcess) Close() error                { return p.f.Close() }
func (p *ptyProcess) Wait() error                 { return p.cmd.Wait() }
func (p *ptyProcess) Resize(cols, rows uint16) error {
	return pty.Setsize(p.f, &pty.Winsize{Rows: rows, Cols: cols})
}

func startProcess(provider string, args []string, cwd string, cols, rows int) (processSession, error) {
	cmd := exec.Command(provider, args...)
	cmd.Dir = cwd
	cmd.Env = append(os.Environ(), "TERM=xterm-256color", "COLORTERM=truecolor", "AI_CLI_HUB=1")
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

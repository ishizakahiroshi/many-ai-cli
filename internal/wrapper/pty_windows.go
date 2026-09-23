//go:build windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"
	"sync"
	"time"

	gopty "github.com/aymanbagabas/go-pty"

	"many-ai-cli/internal/execpath"
)

type conPtyProcess struct {
	pty       gopty.Pty
	cmd       *gopty.Cmd
	closeOnce sync.Once
	waitOnce  sync.Once
	waitDone  chan struct{}
	waitErr   error
}

func (p *conPtyProcess) Read(b []byte) (int, error)  { return p.pty.Read(b) }
func (p *conPtyProcess) Write(b []byte) (int, error) { return p.pty.Write(b) }

// Close is idempotent: the first call closes the PTY master, then kills the
// child only if Wait does not complete within the grace period.
func (p *conPtyProcess) Close() error {
	var err error
	p.closeOnce.Do(func() {
		err = p.pty.Close()

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

// Wait waits for the child process to exit. Errors from Kill (e.g. process
// already exited) are expected and can be safely ignored by callers.
func (p *conPtyProcess) Wait() error {
	p.waitOnce.Do(func() {
		defer close(p.waitDone)
		if p.cmd != nil {
			p.waitErr = p.cmd.Wait()
		}
	})
	<-p.waitDone
	return p.waitErr
}

// exitSignalInfo always reports "no signal" on Windows: exec.ExitError.Sys()
// is not a syscall.WaitStatus on this platform, and Windows process exit
// reporting has no POSIX-signal concept. Kept as a same-named counterpart to
// the Unix implementation in pty_unix.go so classifyExit (wrapper.go) can
// call it without build tags.
func exitSignalInfo(exitErr *exec.ExitError) (bool, string) {
	return false, ""
}

func (p *conPtyProcess) Resize(cols, rows uint16) error {
	return p.pty.Resize(int(cols), int(rows))
}

func startProcess(provider string, customArgv []string, args []string, cwd string, cols, rows int, extraEnv []string, terminalColor string) (processSession, error) {
	cmdName, cmdArgs := resolveCmd(provider, customArgv, args)

	pt, err := gopty.New()
	if err != nil {
		return nil, fmt.Errorf("pty new: %w", err)
	}

	// Resize は cmd.Start の前に行う。Start 後だと Claude Code が誤ったサイズで初期描画してしまう。
	if cols > 0 && rows > 0 {
		_ = pt.Resize(cols, rows)
	}

	cmd := pt.Command(cmdName, cmdArgs...)
	cmd.Dir = cwd
	// 子へ渡す環境の組み立て（TERM/COLORTERM の上書き・色の強制・NO_COLOR の扱い）は
	// childEnv に集約した。方針（force / inherit / off）と実測値、設定画面から変えられる
	// ことは
	// env_color.go のコメント参照。
	cmd.Env = append(childEnv(os.Environ(), terminalColor), extraEnv...)

	if err := cmd.Start(); err != nil {
		_ = pt.Close()
		return nil, fmt.Errorf("start %s: %w", provider, err)
	}

	return &conPtyProcess{pty: pt, cmd: cmd, waitDone: make(chan struct{})}, nil
}

// resolveCmd picks the executable and final argv for this session.
// customArgv (non-nil only for a config.yaml custom_providers entry — see
// wrapper.customProviderFor) takes priority: its first element is the
// executable to resolve (through the same npm-shim unwrapping as any other
// provider, via resolveExecutablePath), the rest is prepended to args.
// Custom providers never reach the shell/copilot special cases below, and
// never fall into the bare exec.LookPath(provider) fallback either —
// provider there is a config.yaml id, not necessarily a real binary name.
func resolveCmd(provider string, customArgv []string, args []string) (string, []string) {
	if len(customArgv) > 0 {
		combined := append(append([]string{}, customArgv[1:]...), args...)
		exePath, err := exec.LookPath(customArgv[0])
		if err != nil {
			return customArgv[0], combined
		}
		return execpath.Resolve(exePath, combined)
	}
	if provider == "shell" {
		return resolveDefaultShell(), args
	}
	if provider == "copilot" {
		if exePath, err := exec.LookPath("copilot"); err == nil {
			return execpath.Resolve(exePath, args)
		}
		if exePath, err := exec.LookPath("gh"); err == nil {
			return execpath.Resolve(exePath, copilotViaGhArgs(args))
		}
		return provider, args
	}
	exePath, err := exec.LookPath(provider)
	if err != nil {
		return provider, args
	}
	return execpath.Resolve(exePath, args)
}

// resolveDefaultShell returns the path to the default interactive shell on
// Windows. Preference order: pwsh.exe → powershell.exe → cmd.exe.
func resolveDefaultShell() string {
	for _, name := range []string{"pwsh.exe", "powershell.exe"} {
		if p, err := exec.LookPath(name); err == nil {
			return p
		}
	}
	comspec := os.Getenv("COMSPEC")
	if comspec != "" {
		if _, err := os.Stat(comspec); err == nil { // #nosec G703 -- comspec は %COMSPEC% 由来の既定シェルパスの存在確認で外部入力ではない
			return comspec
		}
	}
	return `C:\Windows\System32\cmd.exe`
}

// npm shim (.cmd/.ps1/拡張子なし) から実体 .exe を解決するロジックは
// internal/execpath へ移した（internal/hub の CLI バージョン確認も同じ解決を必要と
// するため）。Resolve は execpath.Resolve を参照。

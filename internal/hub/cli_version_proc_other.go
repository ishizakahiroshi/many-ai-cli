//go:build !windows

package hub

import (
	"os/exec"
	"syscall"
)

// cli_version_proc_other.go kills a version-check subprocess and everything
// it started, following internal/headless/proc_other.go: the child gets its
// own process group (Setpgid, so pgid == pid) and the kill goes to the
// negative pid, reaching the whole group.
type cliVersionProcessJob uintptr

// configureCLIVersionProcAttr must be called before cmd.Start.
func configureCLIVersionProcAttr(cmd *exec.Cmd) {
	if cmd == nil {
		return
	}
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
}

// attachCLIVersionProcessJob has nothing to do on this OS: the group was set
// before Start via configureCLIVersionProcAttr.
func attachCLIVersionProcessJob(_ *exec.Cmd) (cliVersionProcessJob, error) { return 0, nil }

func closeCLIVersionProcessJob(_ cliVersionProcessJob) {}

func killCLIVersionProcessTree(cmd *exec.Cmd, _ cliVersionProcessJob) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	if pid := cmd.Process.Pid; pid > 0 {
		_ = syscall.Kill(-pid, syscall.SIGKILL)
	}
	_ = cmd.Process.Kill()
}

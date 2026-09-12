//go:build !windows

package headless

import (
	"os/exec"
	"syscall"
)

// proc_other.go terminates the provider process **and everything it started**
// (元設計 21 節 / 31 節 7), following internal/hub/whisper_job_other.go: the
// child gets its own process group (Setpgid, so pgid == pid) and the kill is
// sent to the negative pid, which reaches the whole group.
//
// Without the group, killing a node shim leaves the real provider binary
// running with a detached stdout — a headless worker nobody can see and nobody
// stopped.

type processJob uintptr

func configureProcCmd(cmd *exec.Cmd) {
	if cmd == nil {
		return
	}
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
}

// attachProcessJob has nothing to do here: the group was set before Start.
func attachProcessJob(_ *exec.Cmd) (processJob, error) { return 0, nil }

func closeProcessJob(_ processJob) {}

// killProcessTree stops the whole process group. The job argument is the
// Windows mechanism and is unused here.
func killProcessTree(cmd *exec.Cmd, _ processJob) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	if pid := cmd.Process.Pid; pid > 0 {
		_ = syscall.Kill(-pid, syscall.SIGKILL)
	}
	_ = cmd.Process.Kill()
}

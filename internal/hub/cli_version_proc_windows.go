//go:build windows

package hub

import (
	"fmt"
	"os/exec"
	"unsafe"

	"golang.org/x/sys/windows"
)

// cli_version_proc_windows.go kills a version-check subprocess and everything
// it started. A provider CLI resolved through the npm .cmd shim (execpath's
// cmd.exe /c fallback) spawns node as a child of cmd.exe; killing only the
// process this file started would leave that child running past the 10s
// budget. Same mechanism internal/headless/proc_windows.go already uses for
// headless runs: a Job Object with KILL_ON_JOB_CLOSE, assigned right after
// Start, terminated explicitly on timeout/cancel.
type cliVersionProcessJob windows.Handle

// configureCLIVersionProcAttr does nothing on Windows: the tree is handled
// after Start via attachCLIVersionProcessJob (Job Object), not SysProcAttr.
func configureCLIVersionProcAttr(_ *exec.Cmd) {}

func attachCLIVersionProcessJob(cmd *exec.Cmd) (cliVersionProcessJob, error) {
	if cmd == nil || cmd.Process == nil {
		return 0, fmt.Errorf("cli_version: missing process")
	}
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return 0, err
	}
	var info windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION
	info.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	if _, err := windows.SetInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)),
		uint32(unsafe.Sizeof(info)),
	); err != nil {
		_ = windows.CloseHandle(job)
		return 0, err
	}
	proc, err := windows.OpenProcess(windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE, false, uint32(cmd.Process.Pid))
	if err != nil {
		_ = windows.CloseHandle(job)
		return 0, err
	}
	defer windows.CloseHandle(proc)
	if err := windows.AssignProcessToJobObject(job, proc); err != nil {
		_ = windows.CloseHandle(job)
		return 0, err
	}
	return cliVersionProcessJob(job), nil
}

func closeCLIVersionProcessJob(job cliVersionProcessJob) {
	if job == 0 {
		return
	}
	_ = windows.CloseHandle(windows.Handle(job))
}

func killCLIVersionProcessTree(cmd *exec.Cmd, job cliVersionProcessJob) {
	if job != 0 {
		_ = windows.TerminateJobObject(windows.Handle(job), 1)
	}
	if cmd == nil || cmd.Process == nil {
		return
	}
	_ = cmd.Process.Kill()
}

//go:build windows

package headless

import (
	"fmt"
	"os/exec"
	"unsafe"

	"golang.org/x/sys/windows"
)

// proc_windows.go terminates the provider process **and everything it started**
// (元設計 21 節 / 31 節 7). A headless provider CLI is usually a node shim that
// launches the real binary, so killing the process many-ai-cli started leaves
// the actual worker running with a detached stdout.
//
// The mechanism is the one this repository already uses for whisper-server
// (internal/hub/whisper_job_windows.go): a Job Object with
// KILL_ON_JOB_CLOSE, assigned right after Start. Closing the handle kills
// every process in the job, whatever the tree looks like by then.

type processJob windows.Handle

// configureProcCmd does nothing on Windows: the tree is handled after Start.
func configureProcCmd(_ *exec.Cmd) {}

func attachProcessJob(cmd *exec.Cmd) (processJob, error) {
	if cmd == nil || cmd.Process == nil {
		return 0, fmt.Errorf("headless: missing process")
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
	return processJob(job), nil
}

// closeProcessJob releases the job, which kills anything still inside it.
func closeProcessJob(job processJob) {
	if job == 0 {
		return
	}
	_ = windows.CloseHandle(windows.Handle(job))
}

// killProcessTree stops the process and everything it started, now. The job is
// terminated explicitly rather than left to the deferred handle close: the
// descendants hold our stdout/stderr pipes open, so as long as they live the
// readers never see EOF and Run — where that close is deferred — never returns.
// A run that has no job (attach failed) falls back to killing the one process.
func killProcessTree(cmd *exec.Cmd, job processJob) {
	if job != 0 {
		_ = windows.TerminateJobObject(windows.Handle(job), 1)
	}
	if cmd == nil || cmd.Process == nil {
		return
	}
	_ = cmd.Process.Kill()
}

//go:build windows

package hub

import (
	"fmt"
	"os/exec"
	"unsafe"

	"golang.org/x/sys/windows"
)

// configureWhisperCmd は Windows では何もしない。子プロセスの取り回しは
// 起動後に attachWhisperProcessJob が JobObject(KILL_ON_JOB_CLOSE) で扱う。
func configureWhisperCmd(_ *exec.Cmd) {}

func attachWhisperProcessJob(cmd *exec.Cmd) (whisperProcessJob, error) {
	if cmd == nil || cmd.Process == nil {
		return 0, fmt.Errorf("missing whisper process")
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
	return whisperProcessJob(job), nil
}

func closeWhisperProcessJob(job whisperProcessJob) {
	if job == 0 {
		return
	}
	_ = windows.CloseHandle(windows.Handle(job))
}

// killWhisperProcess は単体プロセスを終了させる。子孫の確実な掃除は
// JobObject(KILL_ON_JOB_CLOSE) を閉じる closeWhisperProcessJob が担う。
func killWhisperProcess(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	_ = cmd.Process.Kill()
}

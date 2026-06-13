//go:build !windows

package hub

import (
	"os/exec"
	"syscall"
)

// configureWhisperCmd は whisper-server を独立したプロセスグループで起動させる
// （Setpgid: true → 子の pgid == 子の pid）。これにより停止時に
// killWhisperProcess がグループごと kill でき、whisper-server が spawn しうる
// 子まで含めて孤児を残さない。Docker(リモートサーバー) では tini(init: true) が backstop。
func configureWhisperCmd(cmd *exec.Cmd) {
	if cmd == nil {
		return
	}
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
}

func attachWhisperProcessJob(_ *exec.Cmd) (whisperProcessJob, error) {
	return 0, nil
}

func closeWhisperProcessJob(_ whisperProcessJob) {
}

// killWhisperProcess はプロセスグループごと SIGKILL する。Setpgid 済みのため
// 負の pid でグループ全体を対象にできる。失敗時は単体 kill にフォールバック。
func killWhisperProcess(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	pid := cmd.Process.Pid
	if pid > 0 {
		_ = syscall.Kill(-pid, syscall.SIGKILL)
	}
	_ = cmd.Process.Kill()
}

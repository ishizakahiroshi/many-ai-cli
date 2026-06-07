//go:build !windows

package hub

import "os/exec"

func attachWhisperProcessJob(_ *exec.Cmd) (whisperProcessJob, error) {
	return 0, nil
}

func closeWhisperProcessJob(_ whisperProcessJob) {
}

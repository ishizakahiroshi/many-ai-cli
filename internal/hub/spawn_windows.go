//go:build windows

package hub

import (
	"os/exec"
	"syscall"
)

func setCmdSysProcAttr(cmd *exec.Cmd) {
	// CREATE_NEW_PROCESS_GROUP (0x200): UI からの kill-all 等で
	//   process group 単位の制御を可能にする。
	// CREATE_NO_WINDOW (0x08000000): GUI から起動された Hub
	//   (コンソール無し) でも、wrap 子プロセスに新規コンソールを割り当て、
	//   その配下で go-pty / ConPTY が安定して claude.exe / codex を起動できるようにする。
	//   ウィンドウは作成されない (新コンソール内部は HideWindow 相当)。
	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: 0x00000200 | 0x08000000,
		HideWindow:    true,
	}
}

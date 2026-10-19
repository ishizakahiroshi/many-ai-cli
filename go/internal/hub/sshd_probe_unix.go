//go:build !windows
// +build !windows

package hub

import (
	"context"
	"os"
	"os/exec"
	"strings"
)

// osSSHDProber は macOS / Linux での OpenSSH Server 状態検知。
//
// 判定順:
//  1. systemctl があれば `systemctl is-active sshd`（無ければ `ssh`）で
//     active/inactive を見る（Linux systemd 環境の主軸）。
//  2. systemctl 不在（macOS / 非 systemd）なら `pgrep -x sshd` で起動中プロセスを見る。
//  3. いずれでも判定できないが sshd バイナリが存在すれば stopped、無ければ not_installed。
type osSSHDProber struct{}

func (osSSHDProber) probe(ctx context.Context) sshdState {
	if _, err := exec.LookPath("systemctl"); err == nil {
		for _, unit := range []string{"sshd", "ssh"} {
			out, _ := exec.CommandContext(ctx, "systemctl", "is-active", unit).Output()
			switch strings.TrimSpace(string(out)) {
			case "active":
				return sshdState{State: sshdStateRunning}
			case "inactive", "failed", "deactivating":
				return sshdState{State: sshdStateStopped}
			}
			// "unknown" / 空（unit 不在）なら次の unit / フォールバックへ。
		}
	}

	// 非 systemd（macOS 等）: 起動中プロセスを直接見る。
	if _, err := exec.LookPath("pgrep"); err == nil {
		if err := exec.CommandContext(ctx, "pgrep", "-x", "sshd").Run(); err == nil {
			return sshdState{State: sshdStateRunning}
		}
	}

	// プロセスが居ない / 判定不能: バイナリの有無で installed/not_installed を分ける。
	if sshdBinaryPresent() {
		return sshdState{State: sshdStateStopped}
	}
	return sshdState{State: sshdStateNotInstalled}
}

// sshdBinaryPresent は sshd 実行ファイルが存在するか（PATH＋既知パス）を返す。
func sshdBinaryPresent() bool {
	if _, err := exec.LookPath("sshd"); err == nil {
		return true
	}
	for _, p := range []string{
		"/usr/sbin/sshd",
		"/usr/bin/sshd",
		"/sbin/sshd",
		"/usr/local/sbin/sshd",
	} {
		if fi, err := os.Stat(p); err == nil && !fi.IsDir() {
			return true
		}
	}
	return false
}

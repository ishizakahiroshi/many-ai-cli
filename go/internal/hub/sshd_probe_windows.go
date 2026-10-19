//go:build windows
// +build windows

package hub

import (
	"context"
	"os/exec"
	"strings"
)

// osSSHDProber は Windows での OpenSSH Server 状態検知。
// `sc query sshd` でサービス状態を見る（PowerShell 起動コストを避ける）。
//   - サービスが存在しない（"does not exist" / FAILED 1060）→ not_installed
//   - STATE: RUNNING → running
//   - STATE: それ以外（STOPPED 等）→ stopped
//   - 検知不能（exec 失敗・出力解析不能）→ unknown
type osSSHDProber struct{}

func (osSSHDProber) probe(ctx context.Context) sshdState {
	cmd := exec.CommandContext(ctx, "sc", "query", "sshd")
	out, err := cmd.CombinedOutput()
	text := string(out)
	lower := strings.ToLower(text)

	// サービス未登録（OpenSSH Server 機能が未インストール）。
	if strings.Contains(lower, "does not exist") || strings.Contains(lower, "1060") {
		return sshdState{State: sshdStateNotInstalled}
	}
	if err != nil && text == "" {
		// sc 自体が叩けない等 → 検知不能。
		return sshdState{State: sshdStateUnknown}
	}

	// "STATE              : 4  RUNNING" のような行を探す。
	if strings.Contains(lower, "state") {
		if strings.Contains(lower, "running") {
			return sshdState{State: sshdStateRunning}
		}
		return sshdState{State: sshdStateStopped}
	}
	return sshdState{State: sshdStateUnknown}
}

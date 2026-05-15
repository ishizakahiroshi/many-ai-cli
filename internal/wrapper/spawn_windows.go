//go:build windows

package wrapper

import (
	"os/exec"
	"syscall"
)

// prepareHubSpawn は ensureHub が `any-ai-cli serve` を spawn する際の
// プラットフォーム固有設定を行う。Windows では CREATE_NEW_CONSOLE
// (0x00000010) を立てて Hub 専用の表示用ターミナル窓を新規割り当てし、
// banner.go の「閉じないでください」警告を見える形で出す。
// Stdout/Stderr はあえて設定しない（新コンソール側に振り向ける）。
func prepareHubSpawn(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: 0x00000010,
	}
}

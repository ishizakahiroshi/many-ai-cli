//go:build !windows

package wrapper

import (
	"os"
	"os/exec"
)

// prepareHubSpawn は ensureHub が `any-ai-cli serve` を spawn する際の
// プラットフォーム固有設定を行う。Unix では別ウィンドウ概念が無いため、
// 親ターミナルに stdout/stderr を引き継ぎバナーを表示する従来挙動を維持。
func prepareHubSpawn(cmd *exec.Cmd) {
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
}

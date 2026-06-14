//go:build !windows
// +build !windows

package hub

import (
	"os"
	"os/exec"
)

// tailscaleCandidates は macOS / Linux での tailscale CLI 探索候補を返す。
// macOS の Mac App Store / 公式 .app 版は PATH 非通なことがあるため、
// .app 内の実体パスも候補に含める（Linux では空でも PATH 探索で拾える）。
func tailscaleCandidates() []string {
	return []string{
		"/Applications/Tailscale.app/Contents/MacOS/Tailscale",
		"/usr/bin/tailscale",
		"/usr/local/bin/tailscale",
		"/opt/homebrew/bin/tailscale",
	}
}

// tailscaleExe は実際に到達可能な tailscale 実行ファイルパスを返す。
// 見つからなければ空文字（呼び出し側は not_installed として degrade する）。
func tailscaleExe() string {
	if p, err := exec.LookPath("tailscale"); err == nil {
		return p
	}
	for _, p := range tailscaleCandidates() {
		if p == "" {
			continue
		}
		if fi, err := os.Stat(p); err == nil && !fi.IsDir() {
			return p
		}
	}
	return ""
}

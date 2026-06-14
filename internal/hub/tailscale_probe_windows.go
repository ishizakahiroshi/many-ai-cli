//go:build windows
// +build windows

package hub

import (
	"os"
	"os/exec"
	"path/filepath"
)

// tailscaleCandidates は Windows での tailscale CLI 探索候補を返す。
// %ProgramFiles%\Tailscale\tailscale.exe を優先し、なければ PATH 上の
// tailscale(.exe) を使う。PATH 非通の既定インストールに対応する。
func tailscaleCandidates() []string {
	var paths []string
	for _, env := range []string{"ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"} {
		if base := os.Getenv(env); base != "" {
			paths = append(paths, filepath.Join(base, "Tailscale", "tailscale.exe"))
		}
	}
	return paths
}

// tailscaleExe は実際に到達可能な tailscale 実行ファイルパスを返す。
// 見つからなければ空文字（呼び出し側は not_installed として degrade する）。
func tailscaleExe() string {
	for _, p := range tailscaleCandidates() {
		if p == "" {
			continue
		}
		if fi, err := os.Stat(p); err == nil && !fi.IsDir() {
			return p
		}
	}
	if p, err := exec.LookPath("tailscale.exe"); err == nil {
		return p
	}
	if p, err := exec.LookPath("tailscale"); err == nil {
		return p
	}
	return ""
}

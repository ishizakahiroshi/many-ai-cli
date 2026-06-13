//go:build !windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
)

const parentShellEnv = "MANY_AI_CLI_PARENT_SHELL"

// DetectShell は起動元シェルの種別を返す。
// MANY_AI_CLI_PARENT_SHELL がセットされている場合はその値を最優先で返す。
// 取得できない場合は $SHELL 環境変数にフォールバックする。
func DetectShell() string {
	if v := strings.TrimSpace(os.Getenv(parentShellEnv)); v != "" {
		return v
	}

	ppid := os.Getppid()

	switch runtime.GOOS {
	case "linux":
		if name := detectLinuxParentShell(ppid); name != "" {
			return name
		}
	case "darwin":
		out, err := exec.Command("ps", "-p", strconv.Itoa(ppid), "-o", "comm=").Output()
		if err == nil {
			// ログインシェルは先頭に "-" が付く（例: -bash）ので除去する
			name := strings.TrimLeft(strings.TrimSpace(string(out)), "-")
			if name != "" {
				return name
			}
		}
	}

	// 上記で取得できなかった場合は $SHELL（ログインシェル）にフォールバック
	if shell := os.Getenv("SHELL"); shell != "" {
		return filepath.Base(shell)
	}
	return ""
}

func detectLinuxParentShell(ppid int) string {
	var comm string
	if b, err := os.ReadFile(fmt.Sprintf("/proc/%d/comm", ppid)); err == nil {
		comm = strings.TrimLeft(strings.TrimSpace(string(b)), "-")
		if comm != "" && len(comm) < 15 {
			return comm
		}
	}
	if b, err := os.ReadFile(fmt.Sprintf("/proc/%d/cmdline", ppid)); err == nil {
		parts := strings.Split(string(b), "\x00")
		if len(parts) > 0 {
			name := strings.TrimLeft(filepath.Base(parts[0]), "-")
			if name != "" {
				return name
			}
		}
	}
	return comm
}

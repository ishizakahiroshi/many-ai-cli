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
// 親が many-ai-cli（Hub spawn / wrap）のときは祖先を遡る（Windows 実装と同趣旨）。
// 取得できない場合は $SHELL 環境変数にフォールバックする。
func DetectShell() string {
	if v := strings.TrimSpace(os.Getenv(parentShellEnv)); v != "" {
		return v
	}

	const maxAncestorHops = 32
	visited := make(map[int]struct{}, maxAncestorHops)
	ppid := os.Getppid()

	for hops := 0; ppid > 1 && hops < maxAncestorHops; hops++ {
		if _, seen := visited[ppid]; seen {
			break
		}
		visited[ppid] = struct{}{}

		name, next := unixProcessCommAndParent(ppid)
		if name == "" {
			break
		}
		base := filepath.Base(name)
		base = strings.TrimLeft(base, "-")
		if shouldSkipUnixShellProcess(base) {
			if next <= 1 {
				break
			}
			ppid = next
			continue
		}
		return base
	}

	// 上記で取得できなかった場合は $SHELL（ログインシェル）にフォールバック
	if shell := os.Getenv("SHELL"); shell != "" {
		return filepath.Base(shell)
	}
	return ""
}

func shouldSkipUnixShellProcess(comm string) bool {
	switch strings.ToLower(comm) {
	case "many-ai-cli", "many-ai-cli.exe":
		return true
	default:
		return false
	}
}

// unixProcessCommAndParent は pid の comm（または cmdline 先頭）と親 PID を返す。
func unixProcessCommAndParent(pid int) (comm string, parent int) {
	switch runtime.GOOS {
	case "linux":
		comm = detectLinuxParentShell(pid)
		if b, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid)); err == nil {
			// /proc/pid/stat: pid (comm) state ppid ...
			// comm は括弧で囲まれスペースを含み得るので、最後の ')' の後を split する。
			s := string(b)
			if i := strings.LastIndex(s, ")"); i >= 0 && i+2 < len(s) {
				fields := strings.Fields(s[i+2:])
				// fields[0]=state, fields[1]=ppid
				if len(fields) >= 2 {
					if p, err := strconv.Atoi(fields[1]); err == nil {
						parent = p
					}
				}
			}
		}
		return comm, parent
	case "darwin":
		out, err := exec.Command("ps", "-p", strconv.Itoa(pid), "-o", "comm=").Output()
		if err == nil {
			comm = strings.TrimLeft(strings.TrimSpace(string(out)), "-")
		}
		out2, err2 := exec.Command("ps", "-p", strconv.Itoa(pid), "-o", "ppid=").Output()
		if err2 == nil {
			if p, err := strconv.Atoi(strings.TrimSpace(string(out2))); err == nil {
				parent = p
			}
		}
		return comm, parent
	default:
		return "", 0
	}
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

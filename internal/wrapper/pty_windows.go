//go:build windows

package wrapper

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	gopty "github.com/aymanbagabas/go-pty"
)

type conPtyProcess struct {
	pty gopty.Pty
	cmd *gopty.Cmd
}

func (p *conPtyProcess) Read(b []byte) (int, error)  { return p.pty.Read(b) }
func (p *conPtyProcess) Write(b []byte) (int, error) { return p.pty.Write(b) }
func (p *conPtyProcess) Close() error {
	err := p.pty.Close()
	if p.cmd != nil && p.cmd.Process != nil {
		_ = p.cmd.Process.Kill()
	}
	return err
}
func (p *conPtyProcess) Wait() error                 { return p.cmd.Wait() }
func (p *conPtyProcess) Resize(cols, rows uint16) error {
	return p.pty.Resize(int(cols), int(rows))
}

func startProcess(provider string, args []string, cwd string, cols, rows int) (processSession, error) {
	cmdName, cmdArgs := resolveCmd(provider, args)

	pt, err := gopty.New()
	if err != nil {
		return nil, fmt.Errorf("pty new: %w", err)
	}

	// Resize は cmd.Start の前に行う。Start 後だと Claude Code が誤ったサイズで初期描画してしまう。
	if cols > 0 && rows > 0 {
		_ = pt.Resize(cols, rows)
	}

	cmd := pt.Command(cmdName, cmdArgs...)
	cmd.Dir = cwd
	cmd.Env = append(os.Environ(), "TERM=xterm-256color", "COLORTERM=truecolor", "AI_CLI_HUB=1")

	if err := cmd.Start(); err != nil {
		_ = pt.Close()
		return nil, fmt.Errorf("start %s: %w", provider, err)
	}

	return &conPtyProcess{pty: pt, cmd: cmd}, nil
}

func resolveCmd(provider string, args []string) (string, []string) {
	exePath, err := exec.LookPath(provider)
	if err != nil {
		return provider, args
	}
	exePath = sanitizeExecutablePath(exePath)
	lower := strings.ToLower(exePath)
	if strings.HasSuffix(lower, ".cmd") || strings.HasSuffix(lower, ".ps1") || filepath.Ext(lower) == "" {
		// npm の shim (.cmd/.ps1/拡張子なし) から実体 .exe を優先解決する。
		// ConPTY では shim 経由より .exe 直実行の方が安定する。
		if resolved := resolveExeNearShim(exePath); resolved != "" {
			return resolved, args
		}
	}
	if strings.HasSuffix(lower, ".cmd") {
		// .cmd 内の実体 .exe を直接解決して ConPTY で実行する。
		// npm 生成の .cmd は "%dp0%\node_modules\..." 形式で dp0 が末尾 \ を含むため
		// ダブルスラッシュになり cmd.exe が認識できないバグを回避。
		if resolved := resolveExeFromCmd(exePath); resolved != "" {
			return resolved, args
		}
		comspec := os.Getenv("COMSPEC")
		if comspec == "" {
			comspec = `C:\Windows\System32\cmd.exe`
		}
		return comspec, append([]string{"/c", exePath}, args...)
	}
	return exePath, args
}

func resolveExeNearShim(shimPath string) string {
	base := strings.TrimSuffix(shimPath, filepath.Ext(shimPath))
	cmdPath := base + ".cmd"
	if _, err := os.Stat(cmdPath); err == nil {
		if resolved := resolveExeFromCmd(cmdPath); resolved != "" {
			return resolved
		}
	}
	return ""
}

func sanitizeExecutablePath(path string) string {
	p := strings.TrimSpace(path)
	p = strings.TrimPrefix(p, `'`)
	p = strings.TrimSuffix(p, `'`)
	p = strings.TrimPrefix(p, `"`)
	p = strings.TrimSuffix(p, `"`)
	return p
}

// resolveExeFromCmd は npm 生成の .cmd ファイルを解析し、
// 実体 .exe のパスを返す。見つからない場合は空文字を返す。
func resolveExeFromCmd(cmdPath string) string {
	data, err := os.ReadFile(cmdPath)
	if err != nil {
		return ""
	}
	dir := filepath.Dir(cmdPath)
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, `"`) {
			continue
		}
		end := strings.Index(line[1:], `"`)
		if end < 0 {
			continue
		}
		raw := line[1 : end+1]
		raw = strings.ReplaceAll(raw, `%dp0%`, dir)
		raw = strings.ReplaceAll(raw, `%~dp0`, dir)
		raw = sanitizeExecutablePath(raw)
		raw = filepath.Clean(raw)
		if strings.EqualFold(filepath.Ext(raw), ".exe") {
			if resolved := resolveMissingClaudeExe(raw); resolved != "" {
				return resolved
			}
			if _, statErr := os.Stat(raw); statErr == nil {
				return raw
			}
		}
	}
	return ""
}

// resolveMissingClaudeExe recovers from a broken Claude npm shim where
// ...\claude-code\bin\claude.exe is missing and only platform package exe exists.
func resolveMissingClaudeExe(exePath string) string {
	if _, err := os.Stat(exePath); err == nil {
		return exePath
	}
	normalized := strings.ToLower(filepath.Clean(exePath))
	if !strings.HasSuffix(normalized, strings.ToLower(filepath.Join("claude-code", "bin", "claude.exe"))) {
		return ""
	}
	baseDir := filepath.Dir(filepath.Dir(exePath)) // ...\claude-code
	candidates := []string{
		filepath.Join(baseDir, "node_modules", "@anthropic-ai", "claude-code-win32-x64", "claude.exe"),
		filepath.Join(baseDir, "node_modules", "@anthropic-ai", "claude-code-win32-arm64", "claude.exe"),
	}
	for _, c := range candidates {
		if _, err := os.Stat(c); err == nil {
			return c
		}
	}
	return ""
}

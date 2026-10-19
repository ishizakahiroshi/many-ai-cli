//go:build windows

// Package execpath resolves an npm-generated Windows shim (found via
// exec.LookPath) down to the real executable it launches. It exists because
// two independent callers need the exact same rule: internal/wrapper's PTY
// launcher (ConPTY runs more reliably against a .exe than through a
// .cmd/.ps1 shim) and internal/hub's CLI version checker (a plain exec.Cmd
// against a .cmd shim would spawn cmd.exe, which the 10s timeout then has to
// kill through an extra process hop). The logic used to live only in
// internal/wrapper/pty_windows.go; it moved here, unchanged, so the version
// checker does not have to duplicate or import wrapper internals.
package execpath

import (
	"os"
	"path/filepath"
	"strings"
)

// Resolve picks the executable and argv to actually run for exePath (an
// exec.LookPath hit). npm's Windows shims (.cmd/.ps1/no extension) are
// unwrapped to the real .exe when one can be found; otherwise a .cmd falls
// back to "cmd.exe /c <shim> <args>" the same way cmd.exe itself would run
// it. Non-Windows builds (execpath_other.go) return the input unchanged.
func Resolve(exePath string, args []string) (string, []string) {
	exePath = sanitizeExecutablePath(exePath)
	lower := strings.ToLower(exePath)
	if len(args) == 0 && (strings.HasSuffix(lower, ".cmd") || strings.HasSuffix(lower, ".ps1") || filepath.Ext(lower) == "") {
		// npm の shim (.cmd/.ps1/拡張子なし) から実体 .exe を優先解決する。
		// ConPTY では shim 経由より .exe 直実行の方が安定する。
		if resolved := resolveExeNearShim(exePath); resolved != "" {
			return resolved, args
		}
	}
	if strings.HasSuffix(lower, ".cmd") {
		// .cmd 内の実体 .exe を直接解決して実行する。
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

//go:build windows

package setupcmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestStartupFallbackDir(t *testing.T) {
	// 実在しない合成パスを使う（個人の %APPDATA% を書かない）。
	const appData = `C:\fixture\AppData\Roaming`

	got := startupFallbackDir(appData)
	want := filepath.Join(appData, "Microsoft", "Windows", "Start Menu", "Programs", "Startup")
	if got != want {
		t.Errorf("startupFallbackDir = %q, want %q", got, want)
	}
}

// APPDATA が無い環境で %APPDATA% 抜きの相対パスを組んでしまうと、カレント配下に
// Startup フォルダを作りかねない。空なら空を返すことを固定する。
func TestStartupFallbackDirWithoutAppData(t *testing.T) {
	if got := startupFallbackDir(""); got != "" {
		t.Errorf("startupFallbackDir(\"\") = %q, want \"\"", got)
	}
}

func TestWriteWindowsCmd(t *testing.T) {
	dir := t.TempDir()
	exe := `C:\path with space\many-ai-cli.exe`
	path := filepath.Join(dir, "start.cmd")

	if err := writeWindowsCmd(path, exe, "serve --open"); err != nil {
		t.Fatalf("writeWindowsCmd: %v", err)
	}

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	got := string(b)

	wantSubs := []string{
		"@echo off\r\n",
		"cd /d %USERPROFILE%\r\n",
		`call "` + exe + `" serve --open` + "\r\n",
		"pause\r\n",
	}
	for _, s := range wantSubs {
		if !strings.Contains(got, s) {
			t.Errorf("cmd body missing %q\n---\n%s", s, got)
		}
	}
}

//go:build windows

package setupcmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

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

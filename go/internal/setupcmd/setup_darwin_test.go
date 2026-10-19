//go:build darwin

package setupcmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestWriteMacCommand(t *testing.T) {
	dir := t.TempDir()
	exe := "/usr/local/bin/many-ai-cli"
	path := filepath.Join(dir, "Many AI Hub Start.command")

	if err := writeMacCommand(path, exe, "serve --open"); err != nil {
		t.Fatalf("writeMacCommand: %v", err)
	}

	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if info.Mode().Perm() != 0o755 {
		t.Errorf("mode: got %o, want 0755", info.Mode().Perm())
	}

	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	got := string(b)
	for _, s := range []string{"#!/bin/sh\n", `"` + exe + `" serve --open` + "\n"} {
		if !strings.Contains(got, s) {
			t.Errorf("body missing %q\n---\n%s", s, got)
		}
	}
}

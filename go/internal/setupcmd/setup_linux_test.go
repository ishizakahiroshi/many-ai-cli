//go:build linux

package setupcmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestWriteLinuxDesktop(t *testing.T) {
	dir := t.TempDir()
	exe := "/usr/local/bin/many-ai-cli"
	path := filepath.Join(dir, "many-ai-hub-start.desktop")

	if err := writeLinuxDesktop(path, exe, "Many AI Hub Start", "serve --open"); err != nil {
		t.Fatalf("writeLinuxDesktop: %v", err)
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
	wantSubs := []string{
		"[Desktop Entry]\n",
		"Type=Application\n",
		"Name=Many AI Hub Start\n",
		`Exec="` + exe + `" serve --open` + "\n",
		"Terminal=true\n",
	}
	for _, s := range wantSubs {
		if !strings.Contains(got, s) {
			t.Errorf("body missing %q\n---\n%s", s, got)
		}
	}
}

func TestWriteLinuxDesktopEscapesPercentInExec(t *testing.T) {
	path := filepath.Join(t.TempDir(), "start.desktop")
	if err := writeLinuxDesktop(path, "/opt/100%/many-ai-cli", "Many AI", "serve"); err != nil {
		t.Fatal(err)
	}
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), `Exec="/opt/100%%/many-ai-cli" serve`) {
		t.Fatalf("Exec percent was not escaped:\n%s", body)
	}
}

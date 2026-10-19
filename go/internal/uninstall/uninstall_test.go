package uninstall

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

func writeFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

// linkDirForTest points dst at src using whatever directory-link mechanism
// this OS actually grants without asking for elevation, mirroring what
// internal/subscription's seeding creates: a plain os.Symlink on Unix/macOS,
// and — since a directory *symlink* on Windows needs
// SeCreateSymbolicLinkPrivilege, which most accounts lack — a junction made
// via `mklink /J`, the same IO_REPARSE_TAG_MOUNT_POINT reparse point that
// internal/subscription/linkdir_windows.go's createJunction builds directly
// against the Windows API. Using the OS's own mklink here (rather than
// reimplementing or reusing the unexported createJunction) exercises the
// real, independently-produced artifact many-ai-cli's Windows fallback
// actually installs on a user's machine.
func linkDirForTest(src, dst string) error {
	if err := os.Symlink(src, dst); err == nil {
		return nil
	} else if runtime.GOOS != "windows" {
		return err
	}
	out, err := exec.Command("cmd", "/c", "mklink", "/J", dst, src).CombinedOutput()
	if err != nil {
		return fmt.Errorf("mklink /J: %w (%s)", err, out)
	}
	return nil
}

// TestRemoveDataDirDoesNotFollowFileSymlink pins down the file-link half of
// the assumption the whole uninstall path relies on: deleting ~/.many-ai-cli
// must never reach outside it, even though a subscription profile inside it
// can hold a file symlink (a mirrored rule file such as CLAUDE.md) pointing
// at the user's real ~/.claude. Go's os.RemoveAll is documented to not
// follow symlinks, so this should already hold; the test exists so a future
// change that breaks it (e.g. swapping RemoveAll for a hand-rolled walk-and-
// delete) fails loudly instead of quietly deleting someone's real rule file.
func TestRemoveDataDirDoesNotFollowFileSymlink(t *testing.T) {
	outsideRoot := t.TempDir() // stands in for the user's real ~/.claude tree
	realFile := filepath.Join(outsideRoot, "CLAUDE.md")
	const realBody = "# the user's real rules\n"
	writeFile(t, realFile, realBody)

	dataDir := filepath.Join(t.TempDir(), "many-ai-cli-data")
	profileDir := filepath.Join(dataDir, "subscriptions", "claude", "work")
	if err := os.MkdirAll(profileDir, 0o700); err != nil {
		t.Fatal(err)
	}

	linkedFile := filepath.Join(profileDir, "CLAUDE.md")
	if err := os.Symlink(realFile, linkedFile); err != nil {
		t.Skipf("cannot create a file symlink in this environment (Windows needs Developer Mode or admin): %v", err)
	}

	if err := removeDataDir(dataDir); err != nil {
		t.Fatalf("removeDataDir: %v", err)
	}

	if _, err := os.Stat(dataDir); !os.IsNotExist(err) {
		t.Fatalf("dataDir still exists after removeDataDir: err=%v", err)
	}
	body, err := os.ReadFile(realFile)
	if err != nil {
		t.Fatalf("the real file OUTSIDE dataDir was deleted or is unreadable: %v", err)
	}
	if string(body) != realBody {
		t.Fatalf("real file content changed: got %q, want %q", body, realBody)
	}
}

// TestRemoveDataDirDoesNotFollowDirectoryJunction is the directory-link half,
// and the one that actually matters on Windows: subscription seeding never
// makes a directory *symlink* there (it needs a privilege most accounts
// lack), it makes a junction. Historically, os.RemoveAll on Windows has had
// bugs around exactly this shape of reparse point (recursing into what it
// mistook for a plain directory and deleting the linked target's contents),
// so this is verified empirically against a real `mklink /J` junction rather
// than assumed from the symlink documentation alone.
func TestRemoveDataDirDoesNotFollowDirectoryJunction(t *testing.T) {
	outsideRoot := t.TempDir()
	realDir := filepath.Join(outsideRoot, "skills")
	writeFile(t, filepath.Join(realDir, "demo", "SKILL.md"), "# demo skill\n")

	dataDir := filepath.Join(t.TempDir(), "many-ai-cli-data")
	profileDir := filepath.Join(dataDir, "subscriptions", "claude", "work")
	if err := os.MkdirAll(profileDir, 0o700); err != nil {
		t.Fatal(err)
	}

	linkedDir := filepath.Join(profileDir, "skills")
	if err := linkDirForTest(realDir, linkedDir); err != nil {
		t.Skipf("cannot create a directory link in this environment: %v", err)
	}

	if err := removeDataDir(dataDir); err != nil {
		t.Fatalf("removeDataDir: %v", err)
	}

	if _, err := os.Stat(dataDir); !os.IsNotExist(err) {
		t.Fatalf("dataDir still exists after removeDataDir: err=%v", err)
	}
	if _, err := os.Stat(filepath.Join(realDir, "demo", "SKILL.md")); err != nil {
		t.Fatalf("the real directory OUTSIDE dataDir was deleted or is unreadable: %v", err)
	}
}

//go:build windows

package subscription

import (
	"os"
	"path/filepath"
	"testing"
)

// createJunction is the fallback that runs when the account cannot create a
// symlink. On a developer machine os.Symlink usually succeeds, so linkDir never
// reaches it — this test calls it directly, otherwise the reparse-point code
// would only ever be exercised on other people's machines.
func TestCreateJunctionResolvesToTheTarget(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "target")
	if err := os.MkdirAll(filepath.Join(target, "nested"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(target, "nested", "file.txt"), []byte("through the junction\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	link := filepath.Join(root, "link")
	if err := createJunction(target, link); err != nil {
		t.Fatalf("createJunction: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(link, "nested", "file.txt"))
	if err != nil || string(body) != "through the junction\n" {
		t.Fatalf("reading through the junction = %q, %v", body, err)
	}
	// A file written later must appear through the junction: that is the whole
	// reason directories are linked instead of copied.
	if err := os.WriteFile(filepath.Join(target, "added.txt"), []byte("later\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(link, "added.txt")); err != nil {
		t.Fatalf("a file added after the junction was made is not visible: %v", err)
	}
	// Lstat must succeed on the junction itself: that is the whole test
	// PendingSeedEntries runs to decide an entry is already present.
	//
	// It deliberately does not assert os.ModeSymlink. Measured 2026-08-23: a
	// junction read back inside the process that just created it comes out as
	// ModeIrregular, while the same junction reads as ModeSymlink from any
	// other process and matches one made by `mklink /J` byte for byte (verified
	// with `fsutil reparsepoint query`). Asserting the mode here would fail on a
	// correct junction, so the check stays on what the caller actually needs.
	if _, err := os.Lstat(link); err != nil {
		t.Fatalf("Lstat(junction): %v", err)
	}
	if err := os.Remove(link); err != nil {
		t.Fatalf("removing a junction must not need the target: %v", err)
	}
	if _, err := os.Stat(filepath.Join(target, "added.txt")); err != nil {
		t.Fatalf("removing the junction removed the target's contents: %v", err)
	}
}

// A failed junction must leave nothing behind, so the next seeding pass sees
// "missing" and retries instead of "already present".
func TestCreateJunctionLeavesNothingBehindOnFailure(t *testing.T) {
	root := t.TempDir()
	link := filepath.Join(root, "link")
	// A target too long for MAXIMUM_REPARSE_DATA_BUFFER_SIZE (16 KiB): the
	// buffer is built before the directory is touched, so this exercises the
	// rollback path.
	tooLong := root
	for i := 0; i < 2000; i++ {
		tooLong = filepath.Join(tooLong, "segment")
	}
	if err := createJunction(tooLong, link); err == nil {
		t.Fatal("an over-long junction target was accepted")
	}
	if _, err := os.Lstat(link); err == nil {
		t.Fatal("a failed createJunction left a directory behind")
	}
}

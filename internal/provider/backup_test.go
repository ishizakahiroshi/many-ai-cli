package provider

import (
	"os"
	"path/filepath"
	"testing"
)

func TestWriteBytesAtomicReplacesExistingContent(t *testing.T) {
	path := filepath.Join(t.TempDir(), "target.json")
	if err := writeBytesAtomic(path, []byte("first")); err != nil {
		t.Fatal(err)
	}
	if err := writeBytesAtomic(path, []byte("second")); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(raw) != "second" {
		t.Fatalf("content = %q, want %q", raw, "second")
	}
	if _, err := os.Stat(path + atomicReplaceShadowSuffix); !os.IsNotExist(err) {
		t.Fatalf("shadow file was not cleaned up: %v", err)
	}
}

// TestRecoverInterruptedAtomicReplaceRestoresFromShadow pins the fix for the
// crash window writeBytesAtomic's Windows rename-replace fallback cannot
// close by itself: if the process dies between "move the old file aside"
// and "move the new file into place", the target is left missing with its
// last valid content sitting in a ".atomic-replace-shadow" file next to it.
// The old implementation used os.Remove instead of a shadow rename, so that
// same crash window destroyed the old content outright with nothing left to
// recover — this test simulates the new window and checks it self-heals.
func TestRecoverInterruptedAtomicReplaceRestoresFromShadow(t *testing.T) {
	path := filepath.Join(t.TempDir(), "target.json")
	if err := os.WriteFile(path+atomicReplaceShadowSuffix, []byte("last valid content"), 0o600); err != nil {
		t.Fatal(err)
	}
	// Simulate the crash: the shadow exists, but path itself does not yet.
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("test setup: target should not exist yet, stat err = %v", err)
	}
	recoverInterruptedAtomicReplace(path)
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("recovery did not restore target: %v", err)
	}
	if string(raw) != "last valid content" {
		t.Fatalf("recovered content = %q, want %q", raw, "last valid content")
	}
	if _, err := os.Stat(path + atomicReplaceShadowSuffix); !os.IsNotExist(err) {
		t.Fatalf("shadow file should be consumed by the rename, stat err = %v", err)
	}
}

func TestRecoverInterruptedAtomicReplaceIsNoopWhenTargetIsHealthy(t *testing.T) {
	path := filepath.Join(t.TempDir(), "target.json")
	if err := os.WriteFile(path, []byte("healthy"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path+atomicReplaceShadowSuffix, []byte("stale leftover"), 0o600); err != nil {
		t.Fatal(err)
	}
	recoverInterruptedAtomicReplace(path)
	raw, err := os.ReadFile(path)
	if err != nil || string(raw) != "healthy" {
		t.Fatalf("healthy target was disturbed: %q, %v", raw, err)
	}
	// A leftover shadow next to an already-healthy target is cleared so it
	// cannot accidentally "recover" a stale value for a future replace.
	if _, err := os.Stat(path + atomicReplaceShadowSuffix); !os.IsNotExist(err) {
		t.Fatalf("stale shadow next to a healthy target was not cleared: %v", err)
	}
}

func TestWriteBytesAtomicSelfHealsBeforeWriting(t *testing.T) {
	path := filepath.Join(t.TempDir(), "target.json")
	if err := os.WriteFile(path+atomicReplaceShadowSuffix, []byte("interrupted-old-value"), 0o600); err != nil {
		t.Fatal(err)
	}
	// target.json is missing (as if a prior replace crashed mid-way); the
	// very next write should still succeed and leave no shadow behind.
	if err := writeBytesAtomic(path, []byte("new-value")); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(path)
	if err != nil || string(raw) != "new-value" {
		t.Fatalf("content after self-heal + write = %q, %v", raw, err)
	}
}

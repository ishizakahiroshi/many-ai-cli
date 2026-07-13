package hub

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

func runWorktreeTestGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}

func TestNormalWorktreeCreateAndSafeCleanup(t *testing.T) {
	repo := t.TempDir()
	if resolved, err := filepath.EvalSymlinks(repo); err == nil {
		repo = resolved
	}
	runWorktreeTestGit(t, repo, "init")
	runWorktreeTestGit(t, repo, "config", "user.name", "test")
	runWorktreeTestGit(t, repo, "config", "user.email", "test@example.invalid")
	if err := os.WriteFile(filepath.Join(repo, "README.md"), []byte("base\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	runWorktreeTestGit(t, repo, "add", "README.md")
	runWorktreeTestGit(t, repo, "commit", "-m", "base")

	tree, err := prepareNormalWorktree(repo, "review worker", time.Date(2026, 7, 11, 12, 34, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(repo, ".git-worktrees", "review-worker-20260711-1234"); tree.Path != want {
		t.Fatalf("worktree path = %q, want %q", tree.Path, want)
	}
	if _, err := os.Stat(tree.Path); err != nil {
		t.Fatalf("created worktree missing: %v", err)
	}
	if err := os.WriteFile(filepath.Join(tree.Path, "change.md"), []byte("change\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	runWorktreeTestGit(t, tree.Path, "add", "change.md")
	runWorktreeTestGit(t, tree.Path, "commit", "-m", "worktree change")
	if err := cleanupNormalWorktree(tree, worktreeCleanupDelete); err == nil {
		t.Fatal("cleanup should retain an unmerged branch")
	}
	if _, err := os.Stat(tree.Path); err != nil {
		t.Fatalf("unmerged worktree should remain: %v", err)
	}
	runWorktreeTestGit(t, repo, "merge", "--ff-only", tree.Branch)
	if err := cleanupNormalWorktree(tree, worktreeCleanupDelete); err != nil {
		t.Fatalf("cleanup merged clean worktree: %v", err)
	}
	if _, err := os.Stat(tree.Path); !os.IsNotExist(err) {
		t.Fatalf("clean merged worktree should be removed, stat err=%v", err)
	}
}

func TestEffectiveWorktreeCleanupDefaultsToManual(t *testing.T) {
	if got := effectiveWorktreeCleanup(""); got != worktreeCleanupManual {
		t.Fatalf("empty policy = %q, want manual", got)
	}
	if validWorktreeCleanup("discard") {
		t.Fatal("invalid cleanup policy accepted")
	}
}

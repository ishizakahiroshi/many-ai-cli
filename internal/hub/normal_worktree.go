package hub

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	worktreeCleanupDelete = "delete"
	worktreeCleanupKeep   = "keep"
	worktreeCleanupManual = "manual"
)

type normalWorktree struct {
	Path      string
	ParentDir string
	Branch    string
	Created   bool
}

func validWorktreeCleanup(value string) bool {
	return value == "" || value == worktreeCleanupDelete || value == worktreeCleanupKeep || value == worktreeCleanupManual
}

func effectiveWorktreeCleanup(value string) string {
	if value == worktreeCleanupDelete || value == worktreeCleanupKeep {
		return value
	}
	return worktreeCleanupManual
}

// prepareNormalWorktree creates a sibling worktree for an ordinary spawned
// session. Its directory deliberately stays below the repository root so it
// remains easy to discover and prune with standard git commands.
func prepareNormalWorktree(cwd, label string, now time.Time) (normalWorktree, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return normalWorktree{}, fmt.Errorf("parent cwd is not a git repository: %w", err)
	}
	parent := strings.TrimSpace(string(out))
	if parent == "" {
		return normalWorktree{}, fmt.Errorf("git did not return repository root")
	}
	name := safeToken(label)
	stamp := now.Format("20060102-1504")
	root := filepath.Join(parent, ".git-worktrees")
	if err := os.MkdirAll(root, 0o700); err != nil {
		return normalWorktree{}, fmt.Errorf("create worktree root: %w", err)
	}
	path := filepath.Join(root, name+"-"+stamp)
	for n := 2; ; n++ {
		if _, statErr := os.Stat(path); os.IsNotExist(statErr) {
			break
		} else if statErr != nil {
			return normalWorktree{}, fmt.Errorf("check worktree path: %w", statErr)
		}
		path = filepath.Join(root, fmt.Sprintf("%s-%s-%d", name, stamp, n))
	}
	branch := "many-ai/" + strings.TrimSuffix(filepath.Base(path), filepath.Ext(path))
	ctx, cancel = context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "git", "-C", parent, "worktree", "add", "-b", branch, path, "HEAD")
	if out, err := cmd.CombinedOutput(); err != nil {
		return normalWorktree{}, fmt.Errorf("create worktree: %s: %w", strings.TrimSpace(string(out)), err)
	}
	return normalWorktree{Path: path, ParentDir: parent, Branch: branch, Created: true}, nil
}

// cleanupNormalWorktree only removes a clean worktree whose branch is already
// merged into the parent checkout. Anything else is retained so a session
// dismissal can never silently discard work.
func cleanupNormalWorktree(tree normalWorktree, policy string) error {
	if !tree.Created || effectiveWorktreeCleanup(policy) != worktreeCleanupDelete {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if out, err := exec.CommandContext(ctx, "git", "-C", tree.Path, "status", "--porcelain").Output(); err != nil {
		return fmt.Errorf("inspect worktree status: %w", err)
	} else if strings.TrimSpace(string(out)) != "" {
		return fmt.Errorf("worktree retained: uncommitted changes")
	}
	if err := exec.CommandContext(ctx, "git", "-C", tree.ParentDir, "merge-base", "--is-ancestor", tree.Branch, "HEAD").Run(); err != nil {
		return fmt.Errorf("worktree retained: branch %q is not merged", tree.Branch)
	}
	ctx, cancel = context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if out, err := exec.CommandContext(ctx, "git", "-C", tree.ParentDir, "worktree", "remove", tree.Path).CombinedOutput(); err != nil {
		return fmt.Errorf("remove worktree: %s: %w", strings.TrimSpace(string(out)), err)
	}
	return nil
}

package hub

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

var normalWorktreeCreateMu sync.Mutex

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
	normalWorktreeCreateMu.Lock()
	defer normalWorktreeCreateMu.Unlock()
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
	// Keep .git-worktrees/ out of the parent repo's `git status`. The directory
	// holds a git worktree (its own `.git` file), so the parent sees it as an
	// untracked embedded repo and `Commit all` (git add -A) would stage it as a
	// mode-160000 gitlink. info/exclude is a repo-local, uncommitted ignore file,
	// so this works in any user repo and needs no cleanup or next-run recovery
	// (unlike editing the tracked .gitignore, which only helped this repo).
	excludeWorktreeDir(ctx, parent)
	path := filepath.Join(root, name+"-"+stamp)
	for n := 2; ; n++ {
		if _, statErr := os.Stat(path); os.IsNotExist(statErr) {
			break
		} else if statErr != nil {
			return normalWorktree{}, fmt.Errorf("check worktree path: %w", statErr)
		}
		path = filepath.Join(root, fmt.Sprintf("%s-%s-%d", name, stamp, n))
	}
	// filepath.Ext を剥がさない: ラベルにドットがあると Ext がタイムスタンプまで
	// 巻き込んで消し（"a.b-20060102-1504" → ".b-20060102-1504"）、ブランチ名が
	// "many-ai/a" に潰れて連続 spawn が同名衝突で失敗する。safeToken が末尾ドットや
	// ".." を落とすので Base をそのまま使ってよい。
	branch := "many-ai/" + filepath.Base(path)
	ctx, cancel = context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "git", "-C", parent, "worktree", "add", "-b", branch, path, "HEAD")
	if out, err := cmd.CombinedOutput(); err != nil {
		return normalWorktree{}, fmt.Errorf("create worktree: %s: %w", strings.TrimSpace(string(out)), err)
	}
	return normalWorktree{Path: path, ParentDir: parent, Branch: branch, Created: true}, nil
}

// excludeWorktreeDir adds `.git-worktrees/` to the parent repo's info/exclude
// (idempotently) so the sibling worktree directory does not surface as an
// untracked embedded repo in the parent's `git status`. info/exclude is
// repo-local and never committed, so best-effort failure is fine.
func excludeWorktreeDir(ctx context.Context, parent string) {
	const entry = ".git-worktrees/"
	out, err := exec.CommandContext(ctx, "git", "-C", parent, "rev-parse", "--git-common-dir").Output()
	if err != nil {
		return
	}
	common := strings.TrimSpace(string(out))
	if common == "" {
		return
	}
	if !filepath.IsAbs(common) {
		common = filepath.Join(parent, common)
	}
	excludePath := filepath.Join(common, "info", "exclude")
	existing, _ := os.ReadFile(excludePath)
	for _, line := range strings.Split(string(existing), "\n") {
		if strings.TrimSpace(line) == entry {
			return // already excluded
		}
	}
	if err := os.MkdirAll(filepath.Dir(excludePath), 0o755); err != nil {
		return
	}
	prefix := ""
	if len(existing) > 0 && existing[len(existing)-1] != '\n' {
		prefix = "\n"
	}
	f, err := os.OpenFile(excludePath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return
	}
	defer f.Close()
	_, _ = f.WriteString(prefix + entry + "\n")
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

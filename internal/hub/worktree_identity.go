package hub

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

var errWorktreeIdentityMismatch = errors.New("worktree identity mismatch")

const worktreeIdentityTimeout = 3 * time.Second

// validateWorktreeIdentity checks the three facts required before reusing a
// worktree: it belongs to the same Git common directory as the parent, Git
// still registers this exact directory as a worktree, and the checked-out
// branch is the branch the Hub recorded. The check is read-only; it never
// switches branches, resets files, or repairs Git metadata.
func validateWorktreeIdentity(parentCWD, worktreePath, expectedBranch string) error {
	parentCWD = strings.TrimSpace(parentCWD)
	worktreePath = strings.TrimSpace(worktreePath)
	expectedBranch = strings.TrimSpace(expectedBranch)
	if parentCWD == "" {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "parent cwd is empty")
	}
	if worktreePath == "" {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "worktree path is empty")
	}
	if expectedBranch == "" {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "expected branch is empty")
	}
	info, err := os.Stat(worktreePath)
	if err != nil {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "worktree path is unavailable: "+err.Error())
	}
	if !info.IsDir() {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "worktree path is not a directory")
	}

	ctx, cancel := context.WithTimeout(context.Background(), worktreeIdentityTimeout)
	defer cancel()
	parentCommon, err := gitCommonDir(ctx, parentCWD)
	if err != nil {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "parent git common directory unavailable: "+err.Error())
	}
	childCommon, err := gitCommonDir(ctx, worktreePath)
	if err != nil {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "worktree git common directory unavailable: "+err.Error())
	}
	if !sameWorktreePath(parentCommon, childCommon) {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, fmt.Sprintf("git common directory changed: parent=%s worktree=%s", parentCommon, childCommon))
	}
	registered, err := gitWorktreeRegistered(ctx, parentCWD, worktreePath)
	if err != nil {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "worktree registration unavailable: "+err.Error())
	}
	if !registered {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "directory is not registered by the parent repository")
	}
	actualBranch, err := gitWorktreeBranch(ctx, worktreePath)
	if err != nil {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, "checked-out branch unavailable: "+err.Error())
	}
	if actualBranch != expectedBranch {
		return worktreeIdentityMismatch(worktreePath, expectedBranch, fmt.Sprintf("checked-out branch is %q", actualBranch))
	}
	return nil
}

func worktreeIdentityMismatch(path, expectedBranch, detail string) error {
	return fmt.Errorf("%w: path=%s expected_branch=%s: %s", errWorktreeIdentityMismatch, path, expectedBranch, detail)
}

func gitCommonDir(ctx context.Context, cwd string) (string, error) {
	command := func(args ...string) ([]byte, error) {
		return exec.CommandContext(ctx, "git", append([]string{"-C", cwd, "rev-parse"}, args...)...).Output()
	}
	out, err := command("--path-format=absolute", "--git-common-dir")
	if err != nil {
		out, err = command("--git-common-dir")
	}
	if err != nil {
		return "", err
	}
	common := strings.TrimSpace(filepath.FromSlash(string(out)))
	if common == "" {
		return "", errors.New("git returned an empty common directory")
	}
	if !filepath.IsAbs(common) {
		common = filepath.Join(cwd, common)
	}
	resolved, err := filepath.EvalSymlinks(filepath.Clean(common))
	if err != nil {
		return "", err
	}
	return filepath.Clean(resolved), nil
}

func gitWorktreeRegistered(ctx context.Context, parentCWD, target string) (bool, error) {
	targetCanonical, err := canonicalWorktreePath(target)
	if err != nil {
		return false, err
	}
	out, err := exec.CommandContext(ctx, "git", "-C", parentCWD, "worktree", "list", "--porcelain").Output()
	if err != nil {
		return false, err
	}
	for _, line := range strings.Split(string(out), "\n") {
		if !strings.HasPrefix(line, "worktree ") {
			continue
		}
		listed, err := canonicalWorktreePath(strings.TrimSpace(strings.TrimPrefix(line, "worktree ")))
		if err != nil {
			// A stale registration may point at a removed directory. It cannot
			// validate the target, so ignore it and continue looking for a live
			// registration with the same canonical path.
			continue
		}
		if sameWorktreePath(targetCanonical, listed) {
			return true, nil
		}
	}
	return false, nil
}

func gitWorktreeBranch(ctx context.Context, worktreePath string) (string, error) {
	out, err := exec.CommandContext(ctx, "git", "-C", worktreePath, "branch", "--show-current").Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

func canonicalWorktreePath(path string) (string, error) {
	if strings.TrimSpace(path) == "" {
		return "", errors.New("path is empty")
	}
	abs, err := filepath.Abs(filepath.Clean(filepath.FromSlash(path)))
	if err != nil {
		return "", err
	}
	resolved, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return "", err
	}
	return filepath.Clean(resolved), nil
}

func sameWorktreePath(a, b string) bool {
	a = filepath.Clean(a)
	b = filepath.Clean(b)
	if runtime.GOOS == "windows" {
		return strings.EqualFold(a, b)
	}
	return a == b
}

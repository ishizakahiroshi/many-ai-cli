package hub

// relay_worktree.go: the shared worktree of a relay (D-16 / D-17) and the git
// helpers the relay state machine uses by default.
//
// In worktree mode a relay works in one worktree that both of its children
// share: `<parent cwd>/<worktree_dir_root>/<orchestration id>/relay` on the
// branch `many-ai-cli/relay/<orchestration id>`, forked from the parent's
// HEAD at start. The user's own checkout is never touched; the implementer
// commits each C to the relay branch and the user merges that branch when the
// relay is done. The worktree is never removed automatically — the branch is
// the user's result — so removal is a user action (dashboard) or a doctor
// finding (internal/doctor/residue.go), both through cleanupRelayWorktree or
// the git command it wraps.

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

var errRelayNotGit = errors.New("parent cwd is not a git repository; pick same-tree mode or start the relay inside a repository")

// errRelayWorktree wraps a git failure while preparing the relay worktree.
type errRelayWorktree struct{ err error }

func (e errRelayWorktree) Error() string { return "relay worktree: " + e.err.Error() }
func (e errRelayWorktree) Unwrap() error { return e.err }

// relayWorktreeRoot is the absolute root that holds relay worktrees for cwd
// (orchestration.worktree_dir_root, default .many-ai-cli/worktrees).
func relayWorktreeRoot(cwd string, cfg config.OrchestrationConfig) string {
	root := strings.TrimSpace(cfg.WorktreeDirRoot)
	if root == "" {
		root = filepath.Join(".many-ai-cli", "worktrees")
	}
	if !filepath.IsAbs(root) {
		root = filepath.Join(cwd, root)
	}
	return root
}

// prepareRelayWorktree creates (or re-attaches) the relay's shared worktree
// and returns its path, branch and the base commit it was forked from. A
// parent cwd outside a git repository is errRelayNotGit: the caller must not
// fall back to same-tree mode on its own, the user chooses that explicitly.
// An existing worktree directory is reused (restore / resume); base is then
// empty and the caller keeps the base it recorded.
func (s *Server) prepareRelayWorktree(parentCWD, orchestrationID string, cfg config.OrchestrationConfig) (path, branch, base string, err error) {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	topOut, err := exec.CommandContext(ctx, "git", "-C", parentCWD, "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return "", "", "", errRelayNotGit
	}
	top := strings.TrimSpace(string(topOut))
	token := safeToken(orchestrationID)
	root := relayWorktreeRoot(parentCWD, cfg)
	path = filepath.Join(root, token, "relay")
	branch = proto.RelayBranchPrefix + token
	if _, statErr := os.Stat(path); statErr == nil {
		return path, branch, "", nil
	}
	baseOut, err := exec.CommandContext(ctx, "git", "-C", parentCWD, "rev-parse", "HEAD").Output()
	if err != nil {
		return "", "", "", errRelayWorktree{fmt.Errorf("rev-parse HEAD failed (does the repository have a commit?): %w", err)}
	}
	base = strings.TrimSpace(string(baseOut))
	if err := os.MkdirAll(filepath.Dir(path), sessionlog.PrivateDirMode); err != nil {
		return "", "", "", errRelayWorktree{err}
	}
	// Keep the worktree root out of the parent's `git status` (same reasoning
	// as .git-worktrees/ in normal_worktree.go): a nested worktree shows up as
	// an untracked embedded repository otherwise.
	if rel, relErr := filepath.Rel(top, root); relErr == nil && rel != "." && !strings.HasPrefix(rel, "..") {
		excludeGitPath(ctx, top, "/"+filepath.ToSlash(rel)+"/")
	}
	ctx2, cancel2 := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel2()
	// #nosec G702 -- exec.CommandContext は argv 直接渡しで shell を介さない。
	// branch と path は safeToken() 済み、parentCWD は自ホストの session cwd。
	out, err := exec.CommandContext(ctx2, "git", "-C", parentCWD, "worktree", "add", "-b", branch, path, base).CombinedOutput()
	if err == nil {
		return path, branch, base, nil
	}
	msg := strings.TrimSpace(string(out))
	if strings.Contains(msg, "already exists") {
		// The branch survived an earlier worktree removal: attach to it instead
		// of failing, so a relay can be resumed after its worktree was pruned.
		// #nosec G702 -- 上の worktree add と同じ理由。argv 直渡しで shell を介さず、
		// path と branch は safeToken() 由来（[^a-zA-Z0-9._-] を潰し先頭の -_. を落とすので
		// オプションに化けない）、parentCWD は自ホストの session cwd。
		out2, err2 := exec.CommandContext(ctx2, "git", "-C", parentCWD, "worktree", "add", path, branch).CombinedOutput()
		if err2 != nil {
			return "", "", "", errRelayWorktree{fmt.Errorf("%s: %w", strings.TrimSpace(string(out2)), err2)}
		}
		return path, branch, base, nil
	}
	return "", "", "", errRelayWorktree{fmt.Errorf("%s: %w", msg, err)}
}

// cleanupRelayWorktree removes a relay worktree. The branch is kept unless
// keepBranch is false, because the branch is what the user merges. It is not
// called when a relay ends; only a user action or doctor decides that.
func (s *Server) cleanupRelayWorktree(parentCWD, path, branch string, keepBranch bool) error {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if out, err := exec.CommandContext(ctx, "git", "-C", parentCWD, "worktree", "remove", "--force", path).CombinedOutput(); err != nil {
		return fmt.Errorf("remove relay worktree: %s: %w", strings.TrimSpace(string(out)), err)
	}
	if keepBranch || !validRevision(branch) {
		return nil
	}
	if out, err := exec.CommandContext(ctx, "git", "-C", parentCWD, "branch", "-D", branch).CombinedOutput(); err != nil {
		return fmt.Errorf("delete relay branch: %s: %w", strings.TrimSpace(string(out)), err)
	}
	return nil
}

// relayGitHead is the default relayDeps.gitHead.
func relayGitHead(dir string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	out, err := runGit(ctx, dir, "rev-parse", "HEAD")
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// relayGitChangedFiles is the default relayDeps.gitChangedFiles: the number of
// files changed in from..HEAD, or the working-tree changes of
// `git status --short` when from is empty (same-tree mode).
func relayGitChangedFiles(dir, from string) (int, error) {
	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	from = strings.TrimSpace(from)
	var out []byte
	var err error
	if from == "" || !validRevision(from) {
		out, err = runGit(ctx, dir, "status", "--short")
	} else {
		out, err = runGit(ctx, dir, "diff", "--name-only", from+"..HEAD")
	}
	if err != nil {
		return 0, err
	}
	n := 0
	for _, line := range strings.Split(string(out), "\n") {
		if strings.TrimSpace(line) != "" {
			n++
		}
	}
	return n, nil
}

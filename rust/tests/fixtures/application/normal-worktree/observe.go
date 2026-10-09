//go:build ignore

package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func git(dir string, args ...string) string {
	out, err := exec.Command("git", append([]string{"-C", dir}, args...)...).CombinedOutput()
	if err != nil {
		panic(fmt.Sprintf("fixture git: %v: %s", err, out))
	}
	return strings.TrimSpace(string(out))
}
func repo(root, name string) string {
	path := filepath.Join(root, name)
	must(os.MkdirAll(path, 0700))
	git(path, "init", "-q")
	git(path, "config", "user.name", "Synthetic Fixture")
	git(path, "config", "user.email", "fixture@example.invalid")
	git(path, "config", "commit.gpgsign", "false")
	git(path, "config", "core.hooksPath", filepath.Join(root, "hooks"))
	must(os.WriteFile(filepath.Join(path, "base.txt"), []byte("base\n"), 0600))
	git(path, "add", "base.txt")
	git(path, "commit", "-qm", "synthetic base")
	return path
}
func exists(path string) bool { _, err := os.Stat(path); return err == nil }
func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}
func prefix(err error) string {
	if err == nil {
		return ""
	}
	return strings.SplitN(err.Error(), ":", 2)[0]
}
func relative(tree normalWorktree) map[string]any {
	return map[string]any{"path": filepath.ToSlash(strings.TrimPrefix(tree.Path, tree.ParentDir+string(filepath.Separator))), "branch": tree.Branch, "created": tree.Created}
}
func main() {
	root := os.Getenv("NORMAL_WORKTREE_FIXTURE_ROOT")
	if root == "" {
		panic("owned fixture root required")
	}
	now := time.Date(2026, 7, 11, 12, 34, 59, 123, time.FixedZone("fixture", 9*3600))
	result := map[string]any{}
	policies := []any{}
	for _, v := range []string{"", "delete", "keep", "manual", "discard", "DELETE", " delete "} {
		policies = append(policies, map[string]any{"value": v, "valid": validWorktreeCleanup(v), "effective": effectiveWorktreeCleanup(v)})
	}
	result["policies"] = policies
	names := []any{}
	for _, v := range []string{"review worker", "a.b", "..a..b..", "日本語 review", " -_. ", strings.Repeat("a", 79) + ".tail"} {
		names = append(names, map[string]any{"label": v, "safe": safeToken(v)})
	}
	result["names"] = names
	r := repo(root, "repo with space 日本語")
	sub := filepath.Join(r, "subdir")
	must(os.Mkdir(sub, 0700))
	exclusion := filepath.Join(r, ".git", "info", "exclude")
	must(os.WriteFile(exclusion, []byte("# keep this without newline"), 0644))
	first, e := prepareNormalWorktree(sub, "a.b", now)
	must(e)
	second, e := prepareNormalWorktree(r, "a.b", now)
	must(e)
	data, e := os.ReadFile(exclusion)
	must(e)
	result["created"] = []any{relative(first), relative(second)}
	result["exclude"] = string(data)
	result["parent_clean"] = git(r, "status", "--porcelain") == ""
	must(os.WriteFile(filepath.Join(first.Path, "untracked.txt"), []byte("work\n"), 0600))
	result["dirty"] = errText(cleanupNormalWorktree(first, "delete"))
	for _, policy := range []string{"keep", "manual", "", "unknown"} {
		must(cleanupNormalWorktree(first, policy))
	}
	result["retained_policies_keep_path"] = exists(first.Path)
	git(first.Path, "add", "untracked.txt")
	git(first.Path, "commit", "-qm", "synthetic work")
	result["unmerged"] = errText(cleanupNormalWorktree(first, "delete"))
	git(r, "merge", "--ff-only", first.Branch)
	must(cleanupNormalWorktree(first, "delete"))
	must(cleanupNormalWorktree(second, "delete"))
	result["removed_after_merge"] = !exists(first.Path) && !exists(second.Path)
	result["branches_retained"] = git(r, "branch", "--list", "many-ai/*") != ""
	result["root_retained"] = exists(filepath.Join(r, ".git-worktrees"))
	_, e = prepareNormalWorktree(r, "a.b", now)
	result["retained_branch_collision"] = prefix(e)
	result["missing_cleanup"] = prefix(cleanupNormalWorktree(first, "delete"))
	empty := filepath.Join(root, "empty")
	must(os.Mkdir(empty, 0700))
	git(empty, "init", "-q")
	_, e = prepareNormalWorktree(empty, "unborn", now)
	result["unborn_create"] = prefix(e)
	result["unborn_root_retained"] = exists(filepath.Join(empty, ".git-worktrees"))
	nongit := filepath.Join(root, "not-git")
	must(os.Mkdir(nongit, 0700))
	_, e = prepareNormalWorktree(nongit, "no", now)
	result["not_git"] = prefix(e)
	concurrent := repo(root, "concurrent")
	trees := make([]normalWorktree, 2)
	errs := make([]error, 2)
	var wg sync.WaitGroup
	start := make(chan struct{})
	for i := range trees {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			<-start
			trees[i], errs[i] = prepareNormalWorktree(concurrent, "review worker", now)
		}(i)
	}
	close(start)
	wg.Wait()
	paths := []string{}
	for i, tree := range trees {
		must(errs[i])
		paths = append(paths, filepath.Base(tree.Path))
		must(cleanupNormalWorktree(tree, "delete"))
	}
	sort.Strings(paths)
	result["concurrent_names"] = paths
	ignoredRepo := repo(root, "ignored")
	ignored, e := prepareNormalWorktree(ignoredRepo, "worker", now)
	must(e)
	ef, e := os.OpenFile(filepath.Join(ignoredRepo, ".git", "info", "exclude"), os.O_APPEND|os.O_WRONLY, 0644)
	must(e)
	_, e = ef.WriteString("ignored.txt\n")
	must(e)
	must(ef.Close())
	must(os.WriteFile(filepath.Join(ignored.Path, "ignored.txt"), []byte("synthetic ignored file\n"), 0600))
	e = cleanupNormalWorktree(ignored, "delete")
	result["ignored_cleanup_error"] = errText(e)
	result["ignored_removed"] = !exists(ignored.Path)
	lockedRepo := repo(root, "locked")
	locked, e := prepareNormalWorktree(lockedRepo, "worker", now)
	must(e)
	git(lockedRepo, "worktree", "lock", locked.Path)
	result["locked_remove"] = prefix(cleanupNormalWorktree(locked, "delete"))
	result["locked_retained"] = exists(locked.Path)
	git(lockedRepo, "worktree", "unlock", locked.Path)
	must(cleanupNormalWorktree(locked, "delete"))
	blocked := repo(root, "blocked")
	must(os.WriteFile(filepath.Join(blocked, ".git-worktrees"), []byte("ordinary synthetic file"), 0600))
	_, e = prepareNormalWorktree(blocked, "worker", now)
	result["root_create_failure"] = prefix(e)
	invalid := repo(root, "invalid-token")
	_, e = prepareNormalWorktree(invalid, "a..b", now)
	result["invalid_token"] = prefix(e)
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	must(enc.Encode(result))
}

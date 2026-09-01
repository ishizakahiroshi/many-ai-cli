package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// newProjectTestRepo は commit が 1 つある一時リポジトリを作って返す。
// 作り方は normal_worktree_test.go の既存テストに合わせている。
func newProjectTestRepo(t *testing.T) string {
	t.Helper()
	repo := t.TempDir()
	if resolved, err := filepath.EvalSymlinks(repo); err == nil {
		repo = resolved
	}
	runWorktreeTestGit(t, repo, "init")
	runWorktreeTestGit(t, repo, "config", "user.name", "test")
	runWorktreeTestGit(t, repo, "config", "user.email", "test@example.com")
	if err := os.WriteFile(filepath.Join(repo, "README.md"), []byte("base\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	runWorktreeTestGit(t, repo, "add", "README.md")
	runWorktreeTestGit(t, repo, "commit", "-m", "base")
	return repo
}

// samePath はドライブレターの大小など、OS 由来の表記ゆれを吸収して比較する。
func samePath(a, b string) bool {
	return strings.EqualFold(filepath.Clean(a), filepath.Clean(b))
}

func TestGitProjectRootReturnsRepositoryRoot(t *testing.T) {
	repo := newProjectTestRepo(t)

	got, resolved := gitProjectRoot(repo)
	if !samePath(got, repo) {
		t.Fatalf("gitProjectRoot(repo) = %q, want %q", got, repo)
	}
	if !resolved {
		t.Fatal("gitProjectRoot(repo) resolved = false, want true")
	}
}

func TestGitProjectRootFromSubdirectory(t *testing.T) {
	repo := newProjectTestRepo(t)
	sub := filepath.Join(repo, "internal", "deep")
	if err := os.MkdirAll(sub, 0o750); err != nil {
		t.Fatal(err)
	}

	got, resolved := gitProjectRoot(sub)
	if !samePath(got, repo) {
		t.Fatalf("gitProjectRoot(sub) = %q, want %q", got, repo)
	}
	if !resolved {
		t.Fatal("gitProjectRoot(sub) resolved = false, want true")
	}
}

// TestGitProjectRootFromWorktree がこの関数の存在理由。worktree の中から呼んでも
// 本体リポのルートが返らないと、relay の子セッションがサイドバーで親と別の箱へ落ちる。
// --show-toplevel だとここが worktree 自身のパスになって落ちる。
func TestGitProjectRootFromWorktree(t *testing.T) {
	repo := newProjectTestRepo(t)
	tree := filepath.Join(t.TempDir(), "relay")
	runWorktreeTestGit(t, repo, "worktree", "add", "-b", "relay-test", tree)
	t.Cleanup(func() {
		runWorktreeTestGit(t, repo, "worktree", "remove", "--force", tree)
	})

	got, resolved := gitProjectRoot(tree)
	if !resolved {
		t.Fatal("gitProjectRoot(worktree) resolved = false, want true")
	}
	if !samePath(got, repo) {
		t.Fatalf("gitProjectRoot(worktree) = %q, want main repo %q", got, repo)
	}
	if samePath(got, tree) {
		t.Fatalf("gitProjectRoot(worktree) returned the worktree itself (%q); --show-toplevel の挙動に退行している", got)
	}
}

func TestGitProjectRootOutsideRepository(t *testing.T) {
	dir := t.TempDir()

	// git 管理外は「取れなかった」ではなく「管理外だと分かった」なので確定扱い。
	// ここを false にすると、git 管理外の cwd で毎周期 git を叩き続けることになる。
	got, resolved := gitProjectRoot(dir)
	if got != "" {
		t.Fatalf("gitProjectRoot(non-repo) = %q, want empty", got)
	}
	if !resolved {
		t.Fatal("gitProjectRoot(non-repo) resolved = false, want true")
	}
}

func TestGitProjectRootWithMissingOrEmptyPath(t *testing.T) {
	if got, resolved := gitProjectRoot(""); got != "" || !resolved {
		t.Fatalf("gitProjectRoot(\"\") = (%q, %v), want (\"\", true)", got, resolved)
	}
	if got, resolved := gitProjectRoot("   "); got != "" || !resolved {
		t.Fatalf("gitProjectRoot(blank) = (%q, %v), want (\"\", true)", got, resolved)
	}
	missing := filepath.Join(t.TempDir(), "does-not-exist")
	if got, _ := gitProjectRoot(missing); got != "" {
		// 存在しないディレクトリで git が確定を返すかは OS 依存なので値だけ見る。
		t.Fatalf("gitProjectRoot(missing) = %q, want empty", got)
	}
}

package hub

// project_id.go: セッションが属する「本体リポジトリ」の場所を求める。
//
// UI のサイドバーは、この値でカードをまとめる箱（プロジェクト）を作る。以前は cwd の
// 末尾フォルダ名という文字列から箱を推測していたため、relay の共有 worktree で動く子
// セッションが親と別の箱へ落ち、しかも末尾が常に "relay" なので別リポジトリの子まで
// 同じ箱へ同居していた。箱は推測ではなく事実から決める。
//
// 由来: docs/local/plan_sidebar-placement-tree_c1_project-id.md

import (
	"context"
	"os/exec"
	"path/filepath"
	"strings"
)

// gitProjectRoot は cwd が属する本体リポジトリのルートを絶対パスで返す。
// git 管理外・git 未インストール・タイムアウトのいずれでも空文字を返し、呼び出し元を
// 止めない（gitBranch と同じ扱い）。
//
// --show-toplevel ではなく --git-common-dir を使う理由:
//
//	worktree の中で --show-toplevel を叩くと worktree 自身のパスが返るため、本体リポと
//	別物に見える。--git-common-dir は worktree でも本体の .git を指すので、その親を取れば
//	本体リポのルートになる。relay の共有 worktree（relay_worktree.go）も通常 worktree
//	（normal_worktree.go）も、これで本体と同じ値へ揃う。
func gitProjectRoot(cwd string) string {
	if strings.TrimSpace(cwd) == "" {
		return ""
	}
	ctx, cancel := context.WithTimeout(context.Background(), branchLookupTimeout)
	defer cancel()

	// --path-format は git 2.31 以降。使えない場合は素の --git-common-dir へ落とし、
	// 相対パスで返ってきたぶんは cwd を基準に絶対化する。ここで諦めると古い git の
	// 利用者だけ project_id が常に空になり、箱が cwd 由来へ静かに退行する。
	out, err := exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--path-format=absolute", "--git-common-dir").Output()
	if err != nil {
		out, err = exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--git-common-dir").Output()
		if err != nil {
			return ""
		}
	}

	common := strings.TrimSpace(string(out))
	if common == "" {
		return ""
	}
	common = filepath.Clean(common)
	if !filepath.IsAbs(common) {
		common = filepath.Clean(filepath.Join(cwd, common))
	}

	// 通常のリポジトリも worktree も <root>/.git を指す。bare リポジトリはリポジトリ
	// 自身のパスが返るので、その場合はそのまま返す。
	if filepath.Base(common) == ".git" {
		return filepath.Clean(filepath.Dir(common))
	}
	return common
}

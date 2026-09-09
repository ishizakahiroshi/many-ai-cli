package hub

// project_id.go: セッションが属する「本体リポジトリ」の場所を求める。
//
// UI のサイドバーは、この値でカードをまとめる箱（プロジェクト）を作る。以前は cwd の
// 末尾フォルダ名という文字列から箱を推測していたため、relay の共有 worktree で動く子
// セッションが親と別の箱へ落ち、しかも末尾が常に "relay" なので別リポジトリの子まで
// 同じ箱へ同居していた。箱は推測ではなく事実から決める。
//
// 取れなかったときの扱いが要（2026-09-01 追加）:
//
//	「git 管理外だから空」と「タイムアウトしたから空」を呼び出し元が区別できないと、
//	一時的な失敗を「このセッションは git 管理外」として記録してしまう。記録された
//	セッションは箱のキーが cwd 末尾へ落ち、同じリポジトリの他セッションと別の箱に
//	分かれたまま二度と直らない。そのため第 2 戻り値で「答えが確定したか」を返し、
//	確定したときだけ記録させる。
//	（docs/local/bugfix_sidebar-box-splits-on-empty-project-id_2026-09-01.md）
//
// 由来: docs/local/plan_sidebar-placement-tree_c1_project-id.md

import (
	"context"
	"errors"
	"os/exec"
	"path/filepath"
	"strings"
)

// gitProjectRoot は cwd が属する本体リポジトリのルートを絶対パスで返す。
//
// 第 2 戻り値は「この答えが確定したか」。true のときの空文字は「git 管理外だと分かった」、
// false のときの空文字は「取れなかった」で、意味がまったく違う。false を記録してはいけない。
//
// --show-toplevel ではなく --git-common-dir を使う理由:
//
//	worktree の中で --show-toplevel を叩くと worktree 自身のパスが返るため、本体リポと
//	別物に見える。--git-common-dir は worktree でも本体の .git を指すので、その親を取れば
//	本体リポのルートになる。relay の共有 worktree（relay_worktree.go）も通常 worktree
//	（normal_worktree.go）も、これで本体と同じ値へ揃う。
func gitProjectRoot(cwd string) (string, bool) {
	if strings.TrimSpace(cwd) == "" {
		// 問い合わせる先が無い。何度やっても同じなので確定として扱う。
		return "", true
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
			return "", gitLookupAnswered(ctx, err)
		}
	}

	common := strings.TrimSpace(string(out))
	if common == "" {
		// 成功したのに何も返さない git は無い。壊れた出力を確定扱いにしない。
		return "", false
	}
	common = filepath.Clean(common)
	if !filepath.IsAbs(common) {
		common = filepath.Clean(filepath.Join(cwd, common))
	}

	// 通常のリポジトリも worktree も <root>/.git を指す。bare リポジトリはリポジトリ
	// 自身のパスが返るので、その場合はそのまま返す。
	if filepath.Base(common) == ".git" {
		return filepath.Clean(filepath.Dir(common)), true
	}
	return common, true
}

// gitLookupAnswered は「git が答えた末の失敗か」を返す。
//
// 答えた失敗（ここは git 管理外だ・そもそも git が入っていない）は確定でよい。
// 答える前の失敗（タイムアウト・起動できない）は未確定で、後で取り直す価値がある。
// branchLookupTimeout は 250ms しかなく、cold start の git はそれを超えうるので、
// この区別を省くと「たまたま遅かった 1 回」が恒久的な誤りとして焼き付く。
func gitLookupAnswered(ctx context.Context, err error) bool {
	if ctx.Err() != nil {
		return false
	}
	if errors.Is(err, exec.ErrNotFound) {
		return true
	}
	var exit *exec.ExitError
	return errors.As(err, &exit)
}

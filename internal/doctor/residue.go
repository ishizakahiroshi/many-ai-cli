// residue.go implements the "did we leave something behind in the user's
// repository?" check.
//
// The design rule it enforces (moved here from CLAUDE.md on 2026-08-19, since
// anyone adding a feature that writes into a user's files ends up here):
//
// A feature that rewrites a user's file for the duration of a session and
// restores it on exit is broken by design if the restore only lives in a defer
// or a graceful shutdown - a kill skips both. Worse, the next run then reads
// the leftover as if it were the original and writes it back, so a single
// abandoned file never self-heals and becomes permanent. Two paths reached the
// point of being committed to the public repository this way: opencode.json
// (internal/wrapper/opencode_config.go) and the approval-rules block in
// AGENTS.md (internal/hub/approval_rules_state.go).
//
//   - Do not try to raise the hit rate of the cleanup; a kill cannot be caught.
//     Always add a path that reclaims the leftover on the NEXT start.
//   - Reclaiming requires recording what we wrote. If what is on disk differs,
//     somebody else has touched it since: leave it alone.
//   - Put the generated filename in .gitignore too, but .gitignore does not
//     apply to already-tracked files, so anything committed needs git rm.
//   - Anything committed BEFORE the reclaim runs never reaches the reclaim path
//     at all. That is what this file catches. The design rule and this detector
//     are a pair; fixing only one of them never reaches the user.
package doctor

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/wrapper"
)

// many-ai-cli はセッション中だけ利用者の作業フォルダへ書く生成物を 2 種類持つ。
// opencode 用の opencode.json（と排他ロック）と、copilot / cursor-agent / grok 用に
// AGENTS.md へ追記する承認ルールブロック。どちらも後始末が kill で飛ぶと置き去りになる。
//
// 置き去りそのものは「次回起動時に回収する」形になっているが、回収より前に commit
// されたものは回収経路に乗らない（.gitignore は追跡中のファイルに効かず git rm が要る）。
// ここでの検査は、その「もう git へ入ってしまったもの」を利用者に気づかせるためのもの。
//
// 検査は cwd の git リポジトリ 1 本だけを見る。自動削除はせず、対処コマンドの提示までとする。

const residueGitTimeout = 3 * time.Second

// residueState は置き去りの 3 分類。対処が違うので区別する。
type residueState int

const (
	// residueNone: 置き去り無し。doctor は 1 行も出力しない。
	residueNone residueState = iota
	// residueWorktreeOnly: 作業フォルダにだけ在る。次回起動時に自動回収される。
	residueWorktreeOnly
	// residueTracked: git の index / commit に入っている。git rm が要る。
	residueTracked
)

// residueReport は cwd のリポジトリ 1 本ぶんの分類結果。
type residueReport struct {
	// Skipped は git が無い、または cwd が git リポジトリでない場合に立つ。
	// この場合は検査自体を行わない（エラーにはしない）。
	Skipped bool
	Root    string

	// Config は opencode.json の状態。ConfigPermission は many-ai-cli が差し込む
	// permission "*" の値（"ask" / "allow"）で、Config が residueNone なら空。
	Config           residueState
	ConfigPermission string

	// Lock は排他ロックファイルの状態。
	Lock residueState

	// Agents は AGENTS.md 内の承認ルールブロックの状態。
	Agents residueState

	// RelayWorktrees は relay（plan_orchestration-relay-loop.md D-16）が
	// `.many-ai-cli/worktrees/<id>/relay` に作った作業ツリーのうち、relay が
	// 進行中でないもの。relay は自分では消さない（ブランチが成果物なので）ため、
	// 利用者が取り込んだ後に残る。
	RelayWorktrees []relayWorktreeResidue
}

// relayWorktreeResidue は残っている relay 作業ツリー 1 本。
type relayWorktreeResidue struct {
	Path       string
	Branch     string
	Registered bool
}

// residue は doctor の検査 1 本ぶん。置き去りが無ければ空スライスを返し、
// 出力を 1 行も増やさない。
func residue(ctx context.Context, cfg *config.Config) []Check {
	cwd, err := os.Getwd()
	if err != nil {
		return nil
	}
	return residueChecks(classifyResidue(ctx, cwd, hubMaybeRunning(cfg)))
}

// hubMaybeRunning は Hub が動いている「可能性」を返す。
//
// doctor から hub パッケージは参照できない（hub が doctor を import しているため
// 循環になる）ので、port() と同じ手段でポートの占有だけを見る。稼働中セッションが
// 正常に書いた AGENTS.md のブロックを置き去りとして報告しないための保険なので、
// 判定は「疑わしければ報告しない」側に倒してよい。
func hubMaybeRunning(cfg *config.Config) bool {
	if cfg == nil {
		return true
	}
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port))
	if err != nil {
		return true
	}
	_ = ln.Close()
	return false
}

// classifyResidue は cwd のリポジトリを走査して 3 分類の結果を返す。
//
// tracked の判定に作業フォルダの現物ではなく git の index を使うのが要点。
// 稼働中セッションは追跡中の opencode.json を上書きしている最中なので、現物を見ると
// 正常な生成物を置き去りと誤認する。index を見れば「commit / stage された内容」だけを
// 対象にできる。
func classifyResidue(ctx context.Context, cwd string, hubRunning bool) residueReport {
	if _, err := exec.LookPath("git"); err != nil {
		return residueReport{Skipped: true}
	}
	root, ok := gitTopLevel(ctx, cwd)
	if !ok {
		return residueReport{Skipped: true}
	}

	report := residueReport{Root: root}

	configName := wrapper.OpenCodeConfigFileName
	lockName := configName + wrapper.OpenCodeLockSuffix

	// opencode.json: index に many-ai-cli が書く permission "*" が入っていれば tracked。
	if blob, found := gitIndexBlob(ctx, root, configName); found {
		if value, has := openCodeManagedPermission(blob); has {
			report.Config = residueTracked
			report.ConfigPermission = value
		}
	}

	// 排他ロック: 名前が many-ai-cli 固有なので、在るだけで自分の生成物と断定できる。
	lockPath := filepath.Join(root, lockName)
	_, lockTracked := gitIndexBlob(ctx, root, lockName)
	lockStale := fileExists(lockPath) && !wrapper.OpenCodeLockHeldByLiveProcess(lockPath)
	switch {
	case lockTracked:
		report.Lock = residueTracked
	case lockStale:
		report.Lock = residueWorktreeOnly
	}

	// 作業フォルダにだけ在る opencode.json は、利用者自身の設定と区別できない。
	// 死んだセッションのロックが隣に残っているときだけ置き去りとみなす
	// （prepareOpenCodeConfig はロックと対で書くため、両方揃って初めて痕跡になる）。
	if report.Config == residueNone && lockStale {
		if blob, err := os.ReadFile(filepath.Join(root, configName)); err == nil {
			if value, has := openCodeManagedPermission(blob); has {
				report.Config = residueWorktreeOnly
				report.ConfigPermission = value
			}
		}
	}

	// AGENTS.md の承認ルールブロックと委譲案内ブロック。新旧どちらのマーカーにも当たる
	// needle で探す。委譲ブロック（2026-08-29 追加）も同じファイルへ注入されるので、
	// どちらか一方でも残っていれば置き去りとして扱う。
	needles := [][]byte{
		[]byte(wrapper.ApprovalRulesResidueNeedle),
		[]byte(wrapper.DelegationResidueNeedle),
	}
	containsAny := func(blob []byte) bool {
		for _, needle := range needles {
			if bytes.Contains(blob, needle) {
				return true
			}
		}
		return false
	}
	if blob, found := gitIndexBlob(ctx, root, "AGENTS.md"); found && containsAny(blob) {
		report.Agents = residueTracked
	} else if !hubRunning {
		// Hub が動いていないのにブロックが残っている = 回収されていない置き去り。
		// 動いている間は稼働中セッションの正常な注入と区別できないので報告しない。
		if blob, err := os.ReadFile(filepath.Join(root, "AGENTS.md")); err == nil && containsAny(blob) {
			report.Agents = residueWorktreeOnly
		}
	}

	// relay の作業ツリー。進行中（relay.json が非終端）のものは使用中なので出さない。
	report.RelayWorktrees = relayWorktreeLeftovers(ctx, root, relayStateDir())

	return report
}

// relayStateDir は relay.json が置かれる ~/.many-ai-cli/orchestration。
// 読めなければ空（= 進行中判定ができないので、全部を残骸として報告する）。
func relayStateDir() string {
	base, err := config.Dir()
	if err != nil {
		return ""
	}
	return filepath.Join(base, "orchestration")
}

// relayWorktreeLeftovers は `git worktree list --porcelain` から relay ブランチ
// （many-ai-cli/relay/<id>）に載っている作業ツリーを拾い、relay.json の state が
// 終端（completed / stopped）か relay.json が無いものだけを返す。
func relayWorktreeLeftovers(ctx context.Context, root, stateDir string) []relayWorktreeResidue {
	out, err := runResidueGit(ctx, root, "worktree", "list", "--porcelain")
	if err != nil {
		return emptyRelayWorktreeLeftovers(root, nil)
	}
	var found []relayWorktreeResidue
	var cur relayWorktreeResidue
	flush := func() {
		if cur.Path != "" && strings.HasPrefix(cur.Branch, proto.RelayBranchPrefix) && !relayInProgress(stateDir, strings.TrimPrefix(cur.Branch, proto.RelayBranchPrefix)) {
			cur.Registered = true
			found = append(found, cur)
		}
		cur = relayWorktreeResidue{}
	}
	for _, line := range strings.Split(string(out), "\n") {
		line = strings.TrimSpace(line)
		switch {
		case strings.HasPrefix(line, "worktree "):
			flush()
			cur.Path = filepath.FromSlash(strings.TrimPrefix(line, "worktree "))
		case strings.HasPrefix(line, "branch refs/heads/"):
			cur.Branch = strings.TrimPrefix(line, "branch refs/heads/")
		case line == "":
			flush()
		}
	}
	flush()
	return emptyRelayWorktreeLeftovers(root, found)
}

// emptyRelayWorktreeLeftovers finds the partial Windows failure shape where
// git already unregistered a relay worktree but could not remove its directory.
// Such a directory is absent from `git worktree list`, so it needs a lexical
// scan of the default relay root to remain visible to doctor.
func emptyRelayWorktreeLeftovers(root string, registered []relayWorktreeResidue) []relayWorktreeResidue {
	found := append([]relayWorktreeResidue(nil), registered...)
	seen := make(map[string]bool, len(found))
	for _, wt := range found {
		seen[filepath.Clean(wt.Path)] = true
	}
	worktreeRoot := filepath.Join(root, ".many-ai-cli", "worktrees")
	entries, err := os.ReadDir(worktreeRoot)
	if err != nil {
		return found
	}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		path := filepath.Join(worktreeRoot, entry.Name(), "relay")
		if seen[filepath.Clean(path)] {
			continue
		}
		children, readErr := os.ReadDir(path)
		if readErr != nil || len(children) != 0 {
			continue
		}
		found = append(found, relayWorktreeResidue{
			Path:   path,
			Branch: proto.RelayBranchPrefix + entry.Name(),
		})
	}
	return found
}

// relayInProgress は relay.json の state が非終端かを返す。ファイルが無い・読めない
// ときは false（= 残骸として報告する側に倒す）。
func relayInProgress(stateDir, orchestrationID string) bool {
	if stateDir == "" || orchestrationID == "" {
		return false
	}
	data, err := os.ReadFile(filepath.Join(stateDir, orchestrationID, "relay.json"))
	if err != nil {
		return false
	}
	var file struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(data, &file); err != nil {
		return false
	}
	switch file.State {
	case "", "completed", "stopped":
		return false
	default:
		return true
	}
}

func residueChecks(report residueReport) []Check {
	if report.Skipped {
		return nil
	}

	configName := wrapper.OpenCodeConfigFileName
	lockName := configName + wrapper.OpenCodeLockSuffix
	gitRmFix := fmt.Sprintf(
		"git rm --cached %s を実行し、.gitignore に %s* を追加してから commit してください（.gitignore は追跡中のファイルには効かないため git rm が要ります）",
		configName, configName)

	var checks []Check

	switch report.Config {
	case residueTracked:
		if report.ConfigPermission == "allow" {
			// 承認プロンプトが出ない状態が公開されている。他と深刻度が違うので分けて出す。
			checks = append(checks, Check{"residue", Fail, fmt.Sprintf(
				"%s が git に登録されており、承認を全許可にする設定（permission %q: %q）が含まれています。clone した利用者の環境でも承認プロンプトが出なくなります",
				configName, wrapper.OpenCodeConfigPermissionKey, report.ConfigPermission), gitRmFix})
		} else {
			checks = append(checks, Check{"residue", Warn, fmt.Sprintf(
				"%s が git に登録されており、many-ai-cli が書く設定（permission %q: %q）が含まれています。後始末を通らずに終了したセッションの置き去りの可能性があります",
				configName, wrapper.OpenCodeConfigPermissionKey, report.ConfigPermission), gitRmFix})
		}
	case residueWorktreeOnly:
		checks = append(checks, Check{"residue", Warn, fmt.Sprintf(
			"%s が作業フォルダに置き去りになっています（permission %q: %q / git には未登録）",
			configName, wrapper.OpenCodeConfigPermissionKey, report.ConfigPermission),
			"次に opencode セッションを起動すると自動で回収されます。commit しないよう注意してください"})
	}

	switch report.Lock {
	case residueTracked:
		checks = append(checks, Check{"residue", Warn,
			fmt.Sprintf("many-ai-cli の排他ロックファイル %s が git に登録されています", lockName),
			fmt.Sprintf("git rm --cached %s を実行し、.gitignore に %s* を追加してから commit してください", lockName, configName)})
	case residueWorktreeOnly:
		checks = append(checks, Check{"residue", Warn,
			fmt.Sprintf("前回の opencode セッションが後始末を通らずに終了した痕跡が残っています（%s / git には未登録）", lockName),
			"次に opencode セッションを起動すると自動で回収されます。今すぐ消す場合はこのファイルを削除してください"})
	}

	switch report.Agents {
	case residueTracked:
		checks = append(checks, Check{"residue", Warn,
			"AGENTS.md に many-ai-cli の承認ルールブロックが含まれたまま git に登録されています",
			"AGENTS.md から many-ai-cli:approval-rules ブロック（旧名 any-ai-cli 版を含む）を削除して commit してください"})
	case residueWorktreeOnly:
		checks = append(checks, Check{"residue", Warn,
			"AGENTS.md に many-ai-cli の承認ルールブロックが残っています（git には未登録）",
			"Hub を起動すると自動で回収されます。commit しないよう注意してください"})
	}

	for _, wt := range report.RelayWorktrees {
		if !wt.Registered {
			checks = append(checks, Check{"residue", Warn,
				fmt.Sprintf("relay の未登録の空ディレクトリが残っています: %s（途中で片付けに失敗した痕跡）", wt.Path),
				fmt.Sprintf("relay が停止済みであることを確認してから、この空ディレクトリを削除してください: %s", wt.Path)})
			continue
		}
		checks = append(checks, Check{"residue", Warn,
			fmt.Sprintf("relay の作業ツリーが残っています: %s（ブランチ %s。relay は進行中ではありません）", wt.Path, wt.Branch),
			fmt.Sprintf("ブランチを取り込んだら `git -C %s worktree remove %s` で片付けてください（未コミットの変更があると拒否されます。捨ててよければ --force）。ブランチ %s は残るので、不要なら `git branch -D %s`", report.Root, wt.Path, wt.Branch, wt.Branch)})
	}

	return checks
}

// openCodeManagedPermission は opencode.json の中身から many-ai-cli が差し込む
// permission "*" の値を取り出す。利用者自身が書いた opencode.json（permission を
// 持たない、または "*" を持たない）と区別するために中身で判定する。
func openCodeManagedPermission(data []byte) (string, bool) {
	var parsed map[string]any
	if err := json.Unmarshal(data, &parsed); err != nil {
		return "", false
	}
	perm, _ := parsed["permission"].(map[string]any)
	if perm == nil {
		return "", false
	}
	value, ok := perm[wrapper.OpenCodeConfigPermissionKey].(string)
	if !ok {
		return "", false
	}
	return value, true
}

func gitTopLevel(ctx context.Context, cwd string) (string, bool) {
	out, err := runResidueGit(ctx, cwd, "rev-parse", "--show-toplevel")
	if err != nil {
		return "", false
	}
	root := strings.TrimSpace(string(out))
	if root == "" {
		return "", false
	}
	return filepath.Clean(root), true
}

// gitIndexBlob は index に登録されている内容を返す。追跡されていなければ found=false。
// 作業フォルダの現物ではなく index を読むので、稼働中セッションによる一時的な
// 書き換えを拾わない。
func gitIndexBlob(ctx context.Context, root, name string) ([]byte, bool) {
	out, err := runResidueGit(ctx, root, "show", ":"+name)
	if err != nil {
		return nil, false
	}
	return out, true
}

func runResidueGit(ctx context.Context, dir string, args ...string) ([]byte, error) {
	cmdCtx, cancel := context.WithTimeout(ctx, residueGitTimeout)
	defer cancel()
	cmd := exec.CommandContext(cmdCtx, "git", append([]string{"-C", dir}, args...)...)
	return cmd.Output()
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

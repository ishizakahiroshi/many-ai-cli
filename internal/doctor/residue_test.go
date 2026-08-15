package doctor

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/wrapper"
)

// initGitRepo は検査用の一時リポジトリを作る。利用者の global gitconfig に依存すると
// 署名要求などでテストが落ちるため、必要な設定はリポジトリローカルで閉じる。
func initGitRepo(t *testing.T) string {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git がありません")
	}
	dir := t.TempDir()
	// macOS の TempDir は /var -> /private/var の symlink なので、
	// git rev-parse --show-toplevel が返す実体パスと合わせておく。
	if resolved, err := filepath.EvalSymlinks(dir); err == nil {
		dir = resolved
	}
	for _, args := range [][]string{
		{"init"},
		{"config", "user.email", "doctor-test@example.com"},
		{"config", "user.name", "doctor test"},
		{"config", "commit.gpgsign", "false"},
	} {
		cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v: %s", args, err, out)
		}
	}
	return dir
}

func gitCommitAll(t *testing.T, dir, message string) {
	t.Helper()
	for _, args := range [][]string{
		{"add", "-A"},
		{"commit", "-m", message},
	} {
		cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v: %s", args, err, out)
		}
	}
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

// writeStaleLock は「保持プロセスが死んだロック」を作る。PID を書くと実行環境の
// プロセス表に左右されるので、読めない中身 + 古い mtime（mtime フォールバックの
// しきい値超え）で確実に stale にする。
func writeStaleLock(t *testing.T, path string) {
	t.Helper()
	writeFile(t, path, "not-a-pid")
	old := time.Now().Add(-24 * time.Hour)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatalf("chtimes %s: %v", path, err)
	}
}

func lockName() string {
	return wrapper.OpenCodeConfigFileName + wrapper.OpenCodeLockSuffix
}

func TestClassifyResidueNoneOnCleanRepo(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, "README.md"), "# hello\n")
	gitCommitAll(t, dir, "init")

	report := classifyResidue(context.Background(), dir, false)
	if report.Skipped {
		t.Fatal("git リポジトリなのにスキップされました")
	}
	if report.Config != residueNone || report.Lock != residueNone || report.Agents != residueNone {
		t.Fatalf("置き去りが無いのに検出されました: %+v", report)
	}
	if checks := residueChecks(report); len(checks) != 0 {
		t.Fatalf("出力が増えました: %+v", checks)
	}
}

// 利用者自身の opencode.json（permission "*" を持たない）は置き去りではない。
func TestClassifyResidueIgnoresUserOwnedConfig(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, wrapper.OpenCodeConfigFileName),
		`{"model":"anthropic/claude-sonnet-5","permission":{"edit":"ask"}}`)
	gitCommitAll(t, dir, "add user config")

	report := classifyResidue(context.Background(), dir, false)
	if report.Config != residueNone {
		t.Fatalf("利用者自身の設定を置き去りと誤判定しました: %+v", report)
	}
}

func TestClassifyResidueTrackedConfigWithBypass(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, wrapper.OpenCodeConfigFileName),
		`{"permission":{"*":"allow"}}`)
	gitCommitAll(t, dir, "commit residue")

	report := classifyResidue(context.Background(), dir, false)
	if report.Config != residueTracked {
		t.Fatalf("tracked と分類されませんでした: %+v", report)
	}
	if report.ConfigPermission != "allow" {
		t.Fatalf("permission の値が取れていません: %q", report.ConfigPermission)
	}

	checks := residueChecks(report)
	if len(checks) != 1 {
		t.Fatalf("checks の件数が想定外です: %+v", checks)
	}
	// 承認バイパスは他の置き去りと深刻度が違うので FAIL で出す。
	if checks[0].Level != Fail {
		t.Fatalf("承認バイパスが %s で出ています", checks[0].Level)
	}
	if checks[0].Fix == "" {
		t.Fatal("対処コマンドが空です")
	}
}

func TestClassifyResidueTrackedConfigWithAsk(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, wrapper.OpenCodeConfigFileName),
		`{"permission":{"*":"ask"}}`)
	gitCommitAll(t, dir, "commit residue")

	checks := residueChecks(classifyResidue(context.Background(), dir, false))
	if len(checks) != 1 || checks[0].Level != Warn {
		t.Fatalf("ask の置き去りは WARN で 1 件のはずです: %+v", checks)
	}
}

// tracked 判定は作業フォルダの現物ではなく index を見る。稼働中セッションが
// 追跡中の opencode.json を書き換えている最中でも、commit 済みの内容が素であれば
// 置き去りとして報告しない。
func TestClassifyResidueUsesIndexNotWorktree(t *testing.T) {
	dir := initGitRepo(t)
	cfgPath := filepath.Join(dir, wrapper.OpenCodeConfigFileName)
	writeFile(t, cfgPath, `{"model":"anthropic/claude-sonnet-5"}`)
	gitCommitAll(t, dir, "add user config")

	// 稼働中セッションによる書き換えと、その生存ロックを再現する。
	writeFile(t, cfgPath, `{"model":"anthropic/claude-sonnet-5","permission":{"*":"ask"}}`)
	writeFile(t, filepath.Join(dir, lockName()), `{"pid":`+strconv.Itoa(os.Getpid())+`}`)

	report := classifyResidue(context.Background(), dir, true)
	if report.Config != residueNone {
		t.Fatalf("稼働中セッションの生成物を置き去りと報告しました: %+v", report)
	}
	if report.Lock != residueNone {
		t.Fatalf("生存プロセスが保持するロックを置き去りと報告しました: %+v", report)
	}
	if checks := residueChecks(report); len(checks) != 0 {
		t.Fatalf("出力が増えました: %+v", checks)
	}
}

func TestClassifyResidueWorktreeOnlyAfterDeadSession(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, "README.md"), "# hello\n")
	gitCommitAll(t, dir, "init")

	writeFile(t, filepath.Join(dir, wrapper.OpenCodeConfigFileName), `{"permission":{"*":"ask"}}`)
	writeStaleLock(t, filepath.Join(dir, lockName()))

	report := classifyResidue(context.Background(), dir, false)
	if report.Lock != residueWorktreeOnly {
		t.Fatalf("stale ロックが worktree-only と分類されませんでした: %+v", report)
	}
	if report.Config != residueWorktreeOnly {
		t.Fatalf("置き去り config が worktree-only と分類されませんでした: %+v", report)
	}
	checks := residueChecks(report)
	if len(checks) != 2 {
		t.Fatalf("checks の件数が想定外です: %+v", checks)
	}
	for _, check := range checks {
		if check.Level != Warn {
			t.Fatalf("worktree-only は WARN のはずです: %+v", check)
		}
	}
}

// 旧名マーカーだけのブロックも検出できること。新名パターンで grep していたら
// 0 件に見えるケースなので、この 1 本が「旧名側で探している」ことの証明になる。
func TestClassifyResidueDetectsLegacyMarkerOnly(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, "AGENTS.md"),
		"# AGENTS\n\n<!-- any-ai-cli:approval-rules -->\nrules\n<!-- /any-ai-cli:approval-rules -->\n")
	gitCommitAll(t, dir, "commit legacy block")

	report := classifyResidue(context.Background(), dir, false)
	if report.Agents != residueTracked {
		t.Fatalf("旧名マーカーのブロックを検出できませんでした: %+v", report)
	}
}

func TestClassifyResidueDetectsCurrentMarker(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, "AGENTS.md"),
		"# AGENTS\n\n<!-- many-ai-cli:approval-rules -->\nrules\n<!-- /many-ai-cli:approval-rules -->\n")
	gitCommitAll(t, dir, "commit current block")

	report := classifyResidue(context.Background(), dir, false)
	if report.Agents != residueTracked {
		t.Fatalf("新名マーカーのブロックを検出できませんでした: %+v", report)
	}
}

// Hub が動いている間は、稼働中セッションが正常に注入したブロックと区別できないので
// 未追跡の AGENTS.md ブロックを報告しない。
func TestClassifyResidueSkipsAgentsBlockWhileHubRunning(t *testing.T) {
	dir := initGitRepo(t)
	writeFile(t, filepath.Join(dir, "README.md"), "# hello\n")
	gitCommitAll(t, dir, "init")
	writeFile(t, filepath.Join(dir, "AGENTS.md"),
		"<!-- many-ai-cli:approval-rules -->\nrules\n<!-- /many-ai-cli:approval-rules -->\n")

	if report := classifyResidue(context.Background(), dir, true); report.Agents != residueNone {
		t.Fatalf("Hub 稼働中に未追跡ブロックを報告しました: %+v", report)
	}
	if report := classifyResidue(context.Background(), dir, false); report.Agents != residueWorktreeOnly {
		t.Fatalf("Hub 停止中の置き去りブロックを検出できませんでした: %+v", report)
	}
}

// git リポジトリでない場所ではエラーにせずスキップする。
func TestClassifyResidueSkipsOutsideGitRepo(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git がありません")
	}
	dir := t.TempDir()
	report := classifyResidue(context.Background(), dir, false)
	if !report.Skipped {
		t.Fatalf("git リポジトリ外なのにスキップされませんでした: %+v", report)
	}
	if checks := residueChecks(report); len(checks) != 0 {
		t.Fatalf("スキップ時に出力が増えました: %+v", checks)
	}
}

// needle がコメント開始記号を含んでいると旧名・新名の両方には当たらない。
func TestApprovalRulesResidueNeedleMatchesBothMarkers(t *testing.T) {
	needle := wrapper.ApprovalRulesResidueNeedle
	for _, marker := range []string{
		"<!-- many-ai-cli:approval-rules -->",
		"<!-- any-ai-cli:approval-rules -->",
	} {
		if !strings.Contains(marker, needle) {
			t.Fatalf("needle %q が %q に当たりません", needle, marker)
		}
	}
}

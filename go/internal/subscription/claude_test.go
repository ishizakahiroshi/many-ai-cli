package subscription

import (
	"path/filepath"
	"testing"

	"many-ai-cli/internal/config"
)

// C1 完了条件（子 plan: plan_child-launch-prompt-and-trust_c2_trust-store.md
// 内部 C1）: env に CLAUDE_CONFIG_DIR があるとき/無いときで、場所が
// <dir>/.claude.json / <home>/.claude.json になる。
//
// ClaudeStateFileFromEnv は明示した env スライスだけを見る。isolateVendorHome
// で本物の HOME を隔離した上で、さらに os.Setenv 側（プロセス自身の env）は
// 空にしておき、渡した env スライスだけが結果に反映されることを確かめる。
func TestClaudeStateFileFromEnvUsesTheGivenEnvNotTheProcessEnv(t *testing.T) {
	home := isolateVendorHome(t)

	if got, want := ClaudeStateFileFromEnv(nil), filepath.Join(home, ".claude.json"); got != want {
		t.Errorf("no CLAUDE_CONFIG_DIR: got %q, want %q", got, want)
	}

	dir := t.TempDir()
	env := []string{ClaudeConfigDirEnv + "=" + dir}
	if got, want := ClaudeStateFileFromEnv(env), filepath.Join(dir, ".claude.json"); got != want {
		t.Errorf("with CLAUDE_CONFIG_DIR=%s: got %q, want %q", dir, got, want)
	}

	// プロセス自身に CLAUDE_CONFIG_DIR を立てても、渡した env スライスに
	// 無ければ無視される（Hub 自身の env ではなく子の env を見るのが目的）。
	t.Setenv(ClaudeConfigDirEnv, t.TempDir())
	if got, want := ClaudeStateFileFromEnv(env), filepath.Join(dir, ".claude.json"); got != want {
		t.Errorf("process env must not leak in: got %q, want %q", got, want)
	}
}

// 値が空文字列のときは、CLAUDE_CONFIG_DIR が無いのと同じ扱いになる。
func TestClaudeStateFileFromEnvIgnoresEmptyValue(t *testing.T) {
	home := isolateVendorHome(t)

	if got, want := ClaudeStateFileFromEnv([]string{ClaudeConfigDirEnv + "="}), filepath.Join(home, ".claude.json"); got != want {
		t.Errorf("empty value: got %q, want %q", got, want)
	}
}

// profile の子は CLAUDE_CONFIG_DIR が many-ai-cli の subscriptions ツリーの中を
// 指し、子が読むのはその中の .claude.json。env スライス版はこの値を捨てては
// いけない（捨てると信頼を ~/.claude.json へ書き、子の画面には確認が出る）。
// 一方、既定の場所を探す claudeDefaultStateFile は同じ値を捨てる（profile は
// 既定ではない）。2 つの関数の違いをここで固定する。
func TestClaudeStateFileFromEnvKeepsAProfileDir(t *testing.T) {
	home := isolateVendorHome(t)
	dir, err := config.Dir()
	if err != nil {
		t.Fatal(err)
	}
	profileDir := config.DefaultSubscriptionProfileDir(dir, "claude", "synthetic-profile")
	if !insideSubscriptionsTree(profileDir) {
		t.Fatalf("fixture error: %s is not inside the subscriptions tree", profileDir)
	}

	env := []string{ClaudeConfigDirEnv + "=" + profileDir}
	if got, want := ClaudeStateFileFromEnv(env), filepath.Join(profileDir, ".claude.json"); got != want {
		t.Errorf("profile child: got %q, want %q", got, want)
	}

	t.Setenv(ClaudeConfigDirEnv, profileDir)
	if got, want := claudeDefaultStateFile(), filepath.Join(home, ".claude.json"); got != want {
		t.Errorf("claudeDefaultStateFile must ignore a profile dir: got %q, want %q", got, want)
	}
}

func TestEnvSliceValueTakesTheLastMatch(t *testing.T) {
	env := []string{"X=1", "X=2"}
	got, found := envSliceValue(env, "X")
	if !found || got != "2" {
		t.Errorf("envSliceValue = (%q, %v), want (\"2\", true)", got, found)
	}
	if _, found := envSliceValue(env, "Y"); found {
		t.Error("envSliceValue found an absent key")
	}
}

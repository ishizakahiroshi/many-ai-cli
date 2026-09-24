package subscription

import (
	"path/filepath"
	"testing"

	"many-ai-cli/internal/config"
)

// C1 完了条件（子 plan: plan_child-launch-prompt-and-trust_c2_trust-store.md
// 内部 C1）: codex も claude と同様に、env に CODEX_HOME があるとき/無いときで
// 場所が <dir>/config.toml / <home>/.codex/config.toml になる。
func TestCodexConfigFileFromEnvUsesTheGivenEnvNotTheProcessEnv(t *testing.T) {
	home := isolateVendorHome(t)

	if got, want := CodexConfigFileFromEnv(nil), filepath.Join(home, ".codex", "config.toml"); got != want {
		t.Errorf("no CODEX_HOME: got %q, want %q", got, want)
	}

	dir := t.TempDir()
	env := []string{CodexHomeEnv + "=" + dir}
	if got, want := CodexConfigFileFromEnv(env), filepath.Join(dir, "config.toml"); got != want {
		t.Errorf("with CODEX_HOME=%s: got %q, want %q", dir, got, want)
	}

	t.Setenv(CodexHomeEnv, t.TempDir())
	if got, want := CodexConfigFileFromEnv(env), filepath.Join(dir, "config.toml"); got != want {
		t.Errorf("process env must not leak in: got %q, want %q", got, want)
	}
}

func TestCodexConfigFileFromEnvIgnoresEmptyValue(t *testing.T) {
	home := isolateVendorHome(t)

	if got, want := CodexConfigFileFromEnv([]string{CodexHomeEnv + "="}), filepath.Join(home, ".codex", "config.toml"); got != want {
		t.Errorf("empty value: got %q, want %q", got, want)
	}
}

// claude と同じ理由で、profile の子の CODEX_HOME（subscriptions ツリーの中）は
// そのまま使う。既定の場所を探す vendorDefaultDir は同じ値を捨てる。
func TestCodexConfigFileFromEnvKeepsAProfileDir(t *testing.T) {
	home := isolateVendorHome(t)
	dir, err := config.Dir()
	if err != nil {
		t.Fatal(err)
	}
	profileDir := config.DefaultSubscriptionProfileDir(dir, "codex", "synthetic-profile")

	env := []string{CodexHomeEnv + "=" + profileDir}
	if got, want := CodexConfigFileFromEnv(env), filepath.Join(profileDir, "config.toml"); got != want {
		t.Errorf("profile child: got %q, want %q", got, want)
	}

	t.Setenv(CodexHomeEnv, profileDir)
	if got, want := vendorDefaultDir(CodexHomeEnv, ".codex"), filepath.Join(home, ".codex"); got != want {
		t.Errorf("vendorDefaultDir must ignore a profile dir: got %q, want %q", got, want)
	}
}

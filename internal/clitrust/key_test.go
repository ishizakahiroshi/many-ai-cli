package clitrust

import (
	"runtime"
	"testing"

	"many-ai-cli/internal/subscription"
)

// C1 完了条件: claudeKey は D:\Tmp\Sample-Repo → D:/Tmp/Sample-Repo、d:\x → D:/x
// （ドライブ文字だけ大文字化・それ以外の大文字小文字は保持）。
func TestClaudeKeyUppercasesOnlyTheDriveLetter(t *testing.T) {
	cases := map[string]string{
		`D:\Tmp\Sample-Repo`: "D:/Tmp/Sample-Repo",
		`d:\x`:               "D:/x",
		`d:\Tmp\SampleApp`:   "D:/Tmp/SampleApp",
		`D:\work\sampleApp`:  "D:/work/sampleApp",
	}
	for in, want := range cases {
		if got := claudeKey(in); got != want {
			t.Errorf("claudeKey(%q) = %q, want %q", in, got, want)
		}
	}
}

// 2 つのキーが大文字小文字だけで違うとき、claudeKey はそれぞれ別の文字列を
// 返す（Claude 自身のキーがそうなっているため。親 plan の実機記録を参照）。
func TestClaudeKeyIsCaseSensitiveBeyondTheDriveLetter(t *testing.T) {
	a := claudeKey(`D:\work\sampleApp`)
	b := claudeKey(`D:\work\SampleApp`)
	if a == b {
		t.Fatalf("claudeKey collapsed two differently-cased paths into %q", a)
	}
}

// codex のキーは Windows でのみ小文字化が確認されている。それ以外の OS では
// 変換しない（判断ログの「未確認」を、テストでも「確認していない変換はしない」
// という形で表す）。
func TestCodexKeyLowercasesOnWindowsOnly(t *testing.T) {
	in := `D:\Tmp\Sample-Repo`
	got := codexKey(in)
	if runtime.GOOS == "windows" {
		if want := `d:\tmp\sample-repo`; got != want {
			t.Errorf("codexKey(%q) = %q, want %q on windows", in, got, want)
		}
		return
	}
	if got != in {
		t.Errorf("codexKey(%q) = %q, want unchanged on %s", in, got, runtime.GOOS)
	}
}

func TestSupportedListsOnlyClaudeAndCodex(t *testing.T) {
	for _, provider := range []string{"claude", "codex"} {
		if !Supported(provider) {
			t.Errorf("Supported(%q) = false, want true", provider)
		}
	}
	for _, provider := range []string{"copilot", "cursor-agent", "opencode", "grok", "command-code", ""} {
		if Supported(provider) {
			t.Errorf("Supported(%q) = true, want false", provider)
		}
	}
}

// C1 完了条件: env に CLAUDE_CONFIG_DIR がある/ないで場所が変わる。clitrust は
// この解決を subscription.ClaudeStateFileFromEnv に委ねているだけであることを
// 配線のレベルで確かめる（規則そのものの再テストは internal/subscription 側）。
func TestResolveUsesTheChildEnvNotTheProcessEnv(t *testing.T) {
	t.Setenv(subscription.ClaudeConfigDirEnv, "") // Hub 自身の env にはさせない
	dir := t.TempDir()
	_, configPath, _, err := resolve(Target{
		Provider: "claude",
		Env:      []string{subscription.ClaudeConfigDirEnv + "=" + dir},
		Dir:      `D:\x`,
	})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	want := subscription.ClaudeStateFileFromEnv([]string{subscription.ClaudeConfigDirEnv + "=" + dir})
	if configPath != want {
		t.Errorf("configPath = %q, want %q", configPath, want)
	}
}

func TestResolveRejectsUnsupportedProvider(t *testing.T) {
	if _, _, _, err := resolve(Target{Provider: "gemini"}); err == nil {
		t.Fatal("resolve: want error for unsupported provider, got nil")
	}
}

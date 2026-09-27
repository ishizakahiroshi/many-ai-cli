package clitrust

import (
	"runtime"
	"testing"

	"many-ai-cli/internal/subscription"
)

// claudeKey は Claude の lF と同じ: path.normalize のあと Windows だけ "\" を "/"
// にする。ドライブ文字も含め、大文字小文字は変えない（Claude の realpath は
// 渡された綴りを保つ。2026-09-25 に claude.exe 2.1.282 の埋め込み JS と
// Bun 1.3.14 で確認。以前はドライブ文字を大文字にしていた）。
func TestClaudeKeyNormalizesButKeepsCase(t *testing.T) {
	cases := map[string]string{
		"/tmp/Sample-Repo":     "/tmp/Sample-Repo",
		"/tmp/a/../Sample/":    "/tmp/Sample",
		`/tmp/back\slash`:      `/tmp/back\slash`,
		"/tmp//double//slash/": "/tmp/double/slash",
	}
	if runtime.GOOS == "windows" {
		cases = map[string]string{
			`D:\Tmp\Sample-Repo`:   "D:/Tmp/Sample-Repo",
			`d:\x`:                 "d:/x",
			`d:\Tmp\SampleApp\`:    "d:/Tmp/SampleApp",
			`D:\work\a\..\b`:       "D:/work/b",
			`D:/work/forward/`:     "D:/work/forward",
			`D:\work\\double\\sep`: "D:/work/double/sep",
		}
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

// codex のキーは Windows でだけ小文字にする（codex-rs rust-v0.156.1 の
// normalize_project_lookup_key。それ以外の OS では綴りを変えない）。存在しない
// フォルダは正規化できないので、codex と同じく渡された綴りのまま使う。
func TestCodexKeyLowercasesOnWindowsOnly(t *testing.T) {
	in := "/Tmp/No-Such-Sample-Repo"
	if runtime.GOOS == "windows" {
		in = `D:\Tmp\No-Such-Sample-Repo`
	}
	got := codexKey(in)
	if runtime.GOOS == "windows" {
		if want := `d:\tmp\no-such-sample-repo`; got != want {
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
	_, configPath, err := resolve(Target{
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
	if _, _, err := resolve(Target{Provider: "gemini"}); err == nil {
		t.Fatal("resolve: want error for unsupported provider, got nil")
	}
}

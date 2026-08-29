package wrapper

import (
	"os"
	"strings"
	"testing"
)

// env はセッションごとに Hub が決めた値なので、設定ファイルの既定より常に優先される。
// 明示的な "0" が設定既定 true を上書きできることまで固定する（片方向だけだと、
// 「config で常時 ON にしている利用者のセッションを個別に静かにできない」に戻る）。
func TestDelegationPromptEnabledEnvOverridesConfigDefault(t *testing.T) {
	cases := []struct {
		name          string
		env           string
		envSet        bool
		configDefault bool
		want          bool
	}{
		{name: "env on beats config off", env: "1", envSet: true, configDefault: false, want: true},
		{name: "env off beats config on", env: "0", envSet: true, configDefault: true, want: false},
		{name: "unset falls back to config on", envSet: false, configDefault: true, want: true},
		{name: "unset falls back to config off", envSet: false, configDefault: false, want: false},
		{name: "unknown value falls back to config", env: "yes", envSet: true, configDefault: true, want: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.envSet {
				t.Setenv(DelegationEnvName, tc.env)
			} else {
				t.Setenv(DelegationEnvName, "")
				os.Unsetenv(DelegationEnvName)
			}
			if got := DelegationPromptEnabled(tc.configDefault); got != tc.want {
				t.Fatalf("DelegationPromptEnabled(%v) with env=%q set=%v = %v, want %v",
					tc.configDefault, tc.env, tc.envSet, got, tc.want)
			}
		})
	}
}

// 案内に 3 コマンドが揃っていること。AI がこの本文だけを読んで委譲へ辿り着けるかは
// 実機でしか分からないが、コマンド名の脱落は機械で止められる。
func TestDelegationPromptFileCarriesTheSubcommands(t *testing.T) {
	path, cleanup, err := WriteDelegationPromptFile(4242)
	if err != nil {
		t.Fatalf("WriteDelegationPromptFile: %v", err)
	}
	defer cleanup()

	body, err := os.ReadFile(path) // #nosec G304 -- テストが直前に作ったパス
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	text := string(body)
	for _, want := range []string{
		"orchestrate spawn --role",
		"orchestrate send --role",
		"orchestrate relay --plan",
		"many-ai-cli orchestrate --help",
	} {
		if !strings.Contains(text, want) {
			t.Fatalf("委譲案内に %q が無い:\n%s", want, text)
		}
	}
	// 乱発の抑止と、relay を勝手に始めないことは本文の要。落とすと子が増え続ける。
	if !strings.Contains(text, "Do not spawn on your own initiative") {
		t.Fatal("委譲案内から「自分の判断で spawn しない」が落ちている")
	}
	if !strings.Contains(text, "Start a relay only when the user explicitly") {
		t.Fatal("委譲案内から「relay はユーザーが明示したときだけ」が落ちている")
	}

	cleanup()
	if _, statErr := os.Stat(path); !os.IsNotExist(statErr) {
		t.Fatalf("cleanup 後もファイルが残っている: %s (%v)", path, statErr)
	}
}

// provider ごとの渡し方。claude は一時ファイル、grok は本文を値で渡す。
// 対応していない provider へは何も足さない（未対応のフラグを付けて起動を壊さない）。
func TestDelegationProviderArgsPerProvider(t *testing.T) {
	claudeArgs, claudeCleanup, err := DelegationProviderArgs("claude", 7)
	if err != nil {
		t.Fatalf("claude: %v", err)
	}
	if claudeCleanup == nil {
		t.Fatal("claude は一時ファイルを作るので cleanup が要る")
	}
	defer claudeCleanup()
	if len(claudeArgs) != 2 || claudeArgs[0] != "--append-system-prompt-file" {
		t.Fatalf("claude args = %#v", claudeArgs)
	}
	if _, statErr := os.Stat(claudeArgs[1]); statErr != nil {
		t.Fatalf("claude へ渡すファイルが無い: %v", statErr)
	}

	grokArgs, grokCleanup, err := DelegationProviderArgs("grok", 7)
	if err != nil {
		t.Fatalf("grok: %v", err)
	}
	if grokCleanup != nil {
		t.Fatal("grok は本文を値で渡すので一時ファイルを作らない")
	}
	if len(grokArgs) != 2 || grokArgs[0] != "--rules" {
		t.Fatalf("grok args = %#v", grokArgs)
	}
	if !strings.Contains(grokArgs[1], "orchestrate spawn --role") {
		t.Fatalf("grok へ渡す本文が案内になっていない: %q", grokArgs[1])
	}
	// system prompt を置換する --system-prompt-override は使わない（CLI 本体の
	// system prompt を消してしまう）。追記の --rules だけを使う。
	for _, arg := range grokArgs {
		if strings.Contains(arg, "system-prompt-override") {
			t.Fatal("grok で置換系のフラグを使っている")
		}
	}

	for _, p := range []string{"codex", "copilot", "cursor-agent", "opencode", "shell", ""} {
		args, cleanup, err := DelegationProviderArgs(p, 7)
		if err != nil || args != nil || cleanup != nil {
			t.Fatalf("%q には何も足さないはず: args=%#v cleanup=%v err=%v", p, args, cleanup != nil, err)
		}
	}
}

// 責務の境界を機械で固定する。approval-rules.md は「AI と Hub のあいだの書式の
// 約束事」だけを載せる場所で、機能の案内は載せない（version 19 で一度混ぜて 20 で
// 撤去した経緯がある）。委譲の案内はセッション単位の一時ファイル側にある。
func TestApprovalRulesFileCarriesNoDelegationGuidance(t *testing.T) {
	for _, forbidden := range []string{
		"orchestrate spawn",
		"orchestrate send",
		"orchestrate relay",
		"Delegation",
		"委譲",
	} {
		if strings.Contains(rulesFileContent, forbidden) {
			t.Fatalf("approval-rules.md に %q が入っている。機能の案内は delegation.go 側へ置く", forbidden)
		}
	}
}

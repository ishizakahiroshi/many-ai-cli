package wrapper

import (
	"strings"
	"testing"
)

// audit #19: Codex stop-hook / Claude statusLine のコマンド文字列が exe パスを
// 未クォートでシェルへ渡し、スペース入りパスで relay が無言で実行されなくなる
// 問題への回帰テスト。

// usageHookQuotePOSIX が値を POSIX シングルクォートで囲み、埋め込みシングル
// クォートを close/quote/reopen で安全化することを確認する。
func TestUsageHookQuotePOSIX_WrapsAndEscapes(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"C:/dev/foo/many-ai-cli.exe", `'C:/dev/foo/many-ai-cli.exe'`},
		{"C:/Program Files/many-ai-cli/many-ai-cli.exe", `'C:/Program Files/many-ai-cli/many-ai-cli.exe'`},
		{"a'b", `'a'\''b'`},
	}
	for _, c := range cases {
		if got := usageHookQuotePOSIX(c.in); got != c.want {
			t.Errorf("usageHookQuotePOSIX(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// codexStopHookBlock がスペース入り exe パスをシングルクォートで 1 引数化し、
// 続く relay フラグはクォート外に残す（語分割で壊れない）ことを確認する。
func TestCodexStopHookBlock_QuotesSpaceyExePath(t *testing.T) {
	p := UsageHookParams{
		HubURL:    "http://127.0.0.1:47777",
		Token:     "deadbeef",
		SessionID: 7,
		ExePath:   `C:\Program Files\many-ai-cli\many-ai-cli.exe`,
	}
	block := codexStopHookBlock(p)

	// exe パスはシングルクォートで囲まれ、スペースが内側に収まる。
	wantQuoted := `'C:/Program Files/many-ai-cli/many-ai-cli.exe'`
	if !strings.Contains(block, wantQuoted) {
		t.Fatalf("codex block does not quote exe path:\n%s", block)
	}

	// relay の固定フラグはクォートの外（語分割される側）に出る。
	if !strings.Contains(block, "' usage-relay --provider codex") {
		t.Errorf("relay flags not placed outside the quoted exe path:\n%s", block)
	}

	// HubURL / Token / SessionID は引き続き無クォートで埋め込まれる（現行構造維持）。
	for _, frag := range []string{
		"--hub http://127.0.0.1:47777",
		"--token deadbeef",
		"--session 7",
	} {
		if !strings.Contains(block, frag) {
			t.Errorf("expected fragment %q in codex block:\n%s", frag, block)
		}
	}

	// TOML テーブル配列形式・マーカーは維持される。
	for _, frag := range []string{usageHookBlockStart, "[[hooks.Stop]]", "command = ", usageHookBlockEnd} {
		if !strings.Contains(block, frag) {
			t.Errorf("expected structural fragment %q in codex block:\n%s", frag, block)
		}
	}
}

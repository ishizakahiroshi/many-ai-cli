package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// 弾いたときに正解の一覧を返す。AI は provider の綴りを揺らす（Codex / ChatGPT / GPT-5）ので、
// 「invalid provider」とだけ返すと当てずっぽうを繰り返す。
func TestInvalidProviderDetailListsEveryValidProvider(t *testing.T) {
	detail := invalidProviderDetail("ChatGPT")
	if !strings.Contains(detail, `"ChatGPT"`) {
		t.Fatalf("弾いた値がメッセージに無い: %s", detail)
	}
	for _, p := range orchestrationProviders {
		if !strings.Contains(detail, p) {
			t.Fatalf("有効な provider %q がメッセージに無い: %s", p, detail)
		}
	}
	// 綴りは厳密。大文字始まりや別名を通すと、意図しない provider で子が起きる。
	for _, bad := range []string{"Codex", "ChatGPT", "GPT-5", "CLAUDE", "gemini", ""} {
		if validOrchestrationProvider(bad) {
			t.Fatalf("%q を有効な provider として受けている", bad)
		}
	}
}

// --provider の省略時にどこから決まるか。記憶 → 親と同じ → codex の順。
func TestResolveChildProviderFallbackPrefersMemoryThenParent(t *testing.T) {
	s := newTestServer()
	parent := &session{Provider: "claude"}

	// 記憶も無い状態では親と同じになる（以前はここが codex 決め打ちだった）。
	if got := s.resolveChildProviderFallback(parent, "review"); got != "claude" {
		t.Fatalf("記憶が無いときは親と同じはず: got %q", got)
	}

	// その役割の記憶があれば、親より記憶が優先される。
	s.cfg.UserPrefs.Spawn.RoleProvider = map[string]string{"review": "grok"}
	if got := s.resolveChildProviderFallback(parent, "review"); got != "grok" {
		t.Fatalf("役割の記憶が使われていない: got %q", got)
	}
	// 記憶は役割ごと。別の役割へ漏らさない。
	if got := s.resolveChildProviderFallback(parent, "implementation"); got != "claude" {
		t.Fatalf("別の役割へ記憶が漏れている: got %q", got)
	}

	// 壊れた記憶（旧版の値・手編集）は無視して親へ落ちる。
	s.cfg.UserPrefs.Spawn.RoleProvider = map[string]string{"review": "ChatGPT"}
	if got := s.resolveChildProviderFallback(parent, "review"); got != "claude" {
		t.Fatalf("無効な記憶を採用している: got %q", got)
	}

	// 親が子に使えない provider（shell）のときだけ最後の受け皿へ。
	if got := s.resolveChildProviderFallback(&session{Provider: "shell"}, "review"); got != "codex" {
		t.Fatalf("親が shell のときの受け皿が違う: got %q", got)
	}
	if got := s.resolveChildProviderFallback(nil, "review"); got != "codex" {
		t.Fatalf("親が nil のときの受け皿が違う: got %q", got)
	}
}

// サブスクリプションは provider に紐づく。同じ provider なら親から引き継ぎ、
// 違うなら新規セッションパネルがその provider 用に覚えている値を使う。
func TestResolveChildSubscriptionInheritsParentThenSavedDefault(t *testing.T) {
	s := newTestServer()
	parent := &session{Provider: "claude", SubscriptionProfileID: "claude-work"}
	s.cfg.UserPrefs.Spawn.Defaults = map[string]string{
		"subscription_claude": "claude-personal",
		"subscription_codex":  "codex-personal",
	}

	if got := s.resolveChildSubscription(parent, "claude"); got != "claude-work" {
		t.Fatalf("同じ provider なら親の profile を引き継ぐはず: got %q", got)
	}
	if got := s.resolveChildSubscription(parent, "codex"); got != "codex-personal" {
		t.Fatalf("別 provider ではパネルの記憶を使うはず: got %q", got)
	}
	if got := s.resolveChildSubscription(parent, "grok"); got != "" {
		t.Fatalf("記憶が無い provider は空（CLI 既定ログイン）のはず: got %q", got)
	}
	// 親が profile を持たないときは、同じ provider でもパネルの記憶へ落ちる。
	if got := s.resolveChildSubscription(&session{Provider: "claude"}, "claude"); got != "claude-personal" {
		t.Fatalf("親に profile が無いときの解決が違う: got %q", got)
	}
}

// 実際に起動した provider を役割ごとに覚える。承認ダイアログで書き換えた値もここへ来るので、
// 「1 度直せば次から効く」が成立する。
func TestRememberRoleProviderStoresOnlyValidValues(t *testing.T) {
	home := t.TempDir()
	// config.Save は os.UserHomeDir() 配下へ書く。実ユーザーの config.yaml を
	// テストで書き換えないよう、両 OS ぶんの環境変数を temp へ向ける。
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.rememberRoleProvider("review", "grok")
	if got := s.cfg.UserPrefs.Spawn.RoleProvider["review"]; got != "grok" {
		t.Fatalf("記憶されていない: got %q", got)
	}

	// 無効な provider と空の役割は記憶しない（次回も同じ失敗を繰り返さないため）。
	s.rememberRoleProvider("review", "ChatGPT")
	if got := s.cfg.UserPrefs.Spawn.RoleProvider["review"]; got != "grok" {
		t.Fatalf("無効な provider で上書きされた: got %q", got)
	}
	s.rememberRoleProvider("", "codex")
	if _, ok := s.cfg.UserPrefs.Spawn.RoleProvider[""]; ok {
		t.Fatal("空の役割を記憶している")
	}

	// 実ユーザーの config ではなく temp 側へ書かれていること。
	if _, err := os.Stat(filepath.Join(home, ".many-ai-cli", "config.yaml")); err != nil {
		t.Fatalf("temp の config.yaml が作られていない: %v", err)
	}
}

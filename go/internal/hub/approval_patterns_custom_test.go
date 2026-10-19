package hub

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/config"
)

func TestValidCustomApprovalPatternAssetName(t *testing.T) {
	s := newTestServer()
	s.cfg.CustomProviders = config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent"}}

	cases := []struct {
		name string
		want bool
	}{
		{"my-cli.json", true},
		{"unregistered.json", false},
		{"my-cli", false},         // .json 拡張子が無い
		{"../my-cli.json", false}, // パス区切りを含む
		{".my-cli.json", false},   // 先頭ドット
		// built-in は .official/.custom の2階層を持つが custom provider には無い。
		// ".json" だけを剥がすので base は "my-cli.official" になり、登録済み id
		// "my-cli" とは一致しない＝false（built-in 用の階層名を誤って通さない）。
		{"my-cli.official.json", false},
	}
	for _, c := range cases {
		if got := s.validCustomApprovalPatternAssetName(c.name); got != c.want {
			t.Errorf("validCustomApprovalPatternAssetName(%q) = %v, want %v", c.name, got, c.want)
		}
	}
}

func TestSyncCustomApprovalPatternsSkipsEntriesWithoutSource(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.CustomProviders = config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent"}}

	s.syncCustomApprovalPatterns(context.Background())

	if _, err := os.Stat(customApprovalPatternAssetPath("my-cli")); !os.IsNotExist(err) {
		t.Fatalf("approval_pattern_source が空なのにミラーファイルが作られている: err=%v", err)
	}
}

func TestSyncCustomApprovalPatternsWritesFromLocalFileSource(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	// ローカルファイル source は validateSlashCmdSource（readSlashCmdSource が使い回す
	// 既存の検証）により絶対パスかつ ~/.many-ai-cli 配下でなければ拒否される。
	// approval_pattern_source もこの制約をそのまま引き継ぐ（README に明記）。
	cfgDir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(cfgDir, 0o700); err != nil {
		t.Fatalf("cfgDir 作成失敗: %v", err)
	}
	sourcePath := filepath.Join(cfgDir, "my-cli-patterns.md")
	md := "# my-cli approval patterns\n\n- `do you want to continue`\n- `press y to allow`\n"
	if err := os.WriteFile(sourcePath, []byte(md), 0o600); err != nil {
		t.Fatalf("fixture 書き込み失敗: %v", err)
	}

	s := newTestServer()
	s.cfg.CustomProviders = config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent", ApprovalPatternSource: sourcePath}}

	s.syncCustomApprovalPatterns(context.Background())

	got := readCustomApprovalPatternsMirror("my-cli")
	want := []string{"do you want to continue", "press y to allow"}
	if len(got) != len(want) {
		t.Fatalf("読み込まれたパターン数が違う: got %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("パターン内容が違う: got %v, want %v", got, want)
		}
	}
}

func TestSyncCustomApprovalPatternsHandlesUnreadableSourceWithoutPanicking(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	s := newTestServer()
	s.cfg.CustomProviders = config.CustomProviders{{ID: "my-cli", Command: "my-cli --agent", ApprovalPatternSource: filepath.Join(home, "does-not-exist.md")}}

	// fetch 失敗（stat エラー）はログに warn するだけで、既存ファイルが無ければ
	// ミラーも作られない。panic しないことと、404 のままであることを固定する。
	s.syncCustomApprovalPatterns(context.Background())

	if _, err := os.Stat(customApprovalPatternAssetPath("my-cli")); !os.IsNotExist(err) {
		t.Fatalf("fetch 失敗時にミラーファイルが作られている: err=%v", err)
	}
}

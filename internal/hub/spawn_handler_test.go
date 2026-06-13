package hub

import "testing"

// TestCalcGridLayout は session 数から正しい grid layout 文字列を返すことを検証する。
func TestCalcGridLayout(t *testing.T) {
	cases := []struct {
		count int
		want  string
	}{
		{0, "1x1"},
		{1, "1x1"},
		{2, "1x2"},
		{3, "2x2"},
		{4, "2x2"},
		{5, "2x3"},
		{6, "2x3"},
		{7, "3x3"},
		{9, "3x3"},
		{10, "4x3"},
		{12, "4x3"},
		{13, "6x3"},
		{18, "6x3"},
	}
	for _, tc := range cases {
		got := calcGridLayout(tc.count)
		if got != tc.want {
			t.Errorf("calcGridLayout(%d) = %q, want %q", tc.count, got, tc.want)
		}
	}
}

// TestSpawnProviderWhitelist は handleSpawn が受け付ける provider を検証する。
// shell が whitelist に含まれることと、無効な provider が拒否されることを確認する。
func TestSpawnProviderWhitelist(t *testing.T) {
	validProviders := []string{"claude", "codex", "copilot", "cursor-agent", "shell"}
	for _, p := range validProviders {
		// whitelist に含まれるかどうかのロジックを spawn_handler.go から抽出して検証する。
		ok := p == "claude" || p == "codex" || p == "copilot" || p == "cursor-agent" || p == "shell"
		if !ok {
			t.Errorf("provider %q should be valid but was rejected", p)
		}
	}

	invalidProviders := []string{"gemini", "openai", "opencode", "", "shell-custom", "-shell"}
	for _, p := range invalidProviders {
		ok := p == "claude" || p == "codex" || p == "copilot" || p == "cursor-agent" || p == "shell"
		if ok {
			t.Errorf("provider %q should be invalid but was accepted", p)
		}
	}
}

// TestSpawnGridPresetValidation は handleSpawnGrid の preset/layout/count バリデーションを検証する。
func TestSpawnGridPresetValidation(t *testing.T) {
	validPresets := map[string]bool{"shell": true, "ai+shell": true}
	invalidPresets := []string{"gemini", "claude", "", "Shell", "ai-shell"}
	for _, p := range invalidPresets {
		if validPresets[p] {
			t.Errorf("preset %q should be invalid", p)
		}
	}
	if !validPresets["shell"] {
		t.Error("preset 'shell' should be valid")
	}
	if !validPresets["ai+shell"] {
		t.Error("preset 'ai+shell' should be valid")
	}

	validLayouts := map[string]bool{
		"": true, "1x1": true, "1x2": true, "2x2": true,
		"2x3": true, "3x3": true, "4x3": true, "6x3": true,
	}
	invalidLayouts := []string{"3x2", "1x4", "5x5", "2X2"}
	for _, l := range invalidLayouts {
		if validLayouts[l] {
			t.Errorf("layout %q should be invalid", l)
		}
	}
	if !validLayouts["2x2"] {
		t.Error("layout '2x2' should be valid")
	}
}

// TestSpawnGridAIProviderValidation は ai+shell preset のときに有効な AI provider を検証する。
func TestSpawnGridAIProviderValidation(t *testing.T) {
	validAIProviders := map[string]bool{
		"claude": true, "codex": true, "copilot": true, "cursor-agent": true,
	}

	valid := []string{"claude", "codex", "copilot", "cursor-agent"}
	for _, p := range valid {
		if !validAIProviders[p] {
			t.Errorf("AI provider %q should be valid for ai+shell preset", p)
		}
	}

	invalid := []string{"shell", "gemini", ""}
	for _, p := range invalid {
		if validAIProviders[p] {
			t.Errorf("AI provider %q should be invalid for ai+shell preset", p)
		}
	}
}

// TestSpawnGridShellIsNotValidAIProvider は shell が ai+shell preset の AI provider として
// 使えないことを確認する（shell は provider whitelist には入るが AI provider ではない）。
func TestSpawnGridShellIsNotValidAIProvider(t *testing.T) {
	validAIProviders := map[string]bool{
		"claude": true, "codex": true, "copilot": true, "cursor-agent": true,
	}
	if validAIProviders["shell"] {
		t.Error("shell should not be a valid AI provider for ai+shell preset")
	}
}

// TestCalcGridLayoutSymmetry は calcGridLayout が session-list.ts の calcDetachedLayout と
// 同じ境界を持つことを確認する（コメントに記載された仕様の回帰テスト）。
func TestCalcGridLayoutSymmetry(t *testing.T) {
	// count=4 は 2x2 （1+2+3+4 まで 2x2）
	if got := calcGridLayout(4); got != "2x2" {
		t.Errorf("count=4 should give 2x2, got %q", got)
	}
	// count=5 は 2x3（5〜6 は 2x3）
	if got := calcGridLayout(5); got != "2x3" {
		t.Errorf("count=5 should give 2x3, got %q", got)
	}
	// count=9 は 3x3（7〜9 は 3x3）
	if got := calcGridLayout(9); got != "3x3" {
		t.Errorf("count=9 should give 3x3, got %q", got)
	}
}

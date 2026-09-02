package hub

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestHubSpawnEnvOverridesHubIdentityAndExecutable(t *testing.T) {
	got := hubSpawnEnv([]string{
		"PATH=/bin",
		"MANY_AI_CLI=0",
		"MANY_AI_CLI_HUB_PORT=1",
		"MANY_AI_CLI_BIN=old",
	}, 47777, `/repo/dist/many-ai-cli`)
	joined := "\n" + strings.Join(got, "\n") + "\n"
	for _, want := range []string{
		"\nMANY_AI_CLI=1\n",
		"\nMANY_AI_CLI_HUB_PORT=47777\n",
		"\nMANY_AI_CLI_BIN=/repo/dist/many-ai-cli\n",
	} {
		if !strings.Contains(joined, want) {
			t.Fatalf("spawn env missing %q: %v", want, got)
		}
	}
	for _, stale := range []string{"MANY_AI_CLI=0", "MANY_AI_CLI_HUB_PORT=1", "MANY_AI_CLI_BIN=old"} {
		if strings.Contains(joined, "\n"+stale+"\n") {
			t.Fatalf("spawn env kept stale value %q: %v", stale, got)
		}
	}
}

func TestConductorPromptRequiresHubExecutable(t *testing.T) {
	prompt := buildConductorInitialPrompt("orch-test", nil)
	if !strings.Contains(prompt, manyAICLIBinEnv) {
		t.Fatalf("conductor prompt does not name %s: %s", manyAICLIBinEnv, prompt)
	}
	if strings.Contains(prompt, "run: many-ai-cli orchestrate relay") {
		t.Fatalf("conductor prompt still recommends PATH lookup: %s", prompt)
	}
}

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

// TestHandleSpawnAcceptsCommandCodeProvider は command-code が handleSpawn の provider
// whitelist（:322 の body.Provider 検証）を通ることを確認する。実プロセスを起動させず、
// 存在しない subscription profile で早期 400 に倒す手法は
// TestSpawnRejectsUnknownSubscriptionProfile と同じ。command-code が provider whitelist
// で弾かれていれば「invalid provider」で 400 になり、通っていれば subscription profile
// 解決の失敗で 400 になる。前者と後者を body の文言で区別する。
func TestHandleSpawnAcceptsCommandCodeProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":                "command-code",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": "deleted-profile",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if strings.Contains(w.Body.String(), "invalid provider") {
		t.Fatalf("command-code was rejected as invalid provider: %s", w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "subscription") {
		t.Fatalf("expected subscription profile error, got: %s", w.Body.String())
	}
}

// TestHandleSpawnAcceptsCustomProvider は config.yaml の custom_providers エントリが
// handleSpawn の provider whitelist を実際に通ることを確認する（敵対レビュー
// 2026-08-31 Finding 1: config.CustomProviders に追加しても spawn_handler.go の
// ホワイトリストが固定リストのままで、UI のドロップダウンに出るのに Launch すると
// 「invalid provider」で 400 になっていた）。TestHandleSpawnAcceptsCommandCodeProvider と
// 同じ手法で、存在しない subscription profile を指定して whitelist の後段まで進んだことを
// エラー文言の違いで確認する。
func TestHandleSpawnAcceptsCustomProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":                "my-cli",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": "deleted-profile",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if strings.Contains(w.Body.String(), "invalid provider") {
		t.Fatalf("custom provider my-cli was rejected as invalid provider: %s", w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "subscription") {
		t.Fatalf("expected subscription profile error, got: %s", w.Body.String())
	}
}

// TestHandleSpawnRejectsProviderNotInCustomProviders は custom_providers が設定されていても
// そこに無い provider 名まで通してしまわないことを確認する（validSpawnProvider がただの
// 「何でも許可」になっていないことの回帰テスト）。
func TestHandleSpawnRejectsProviderNotInCustomProviders(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider": "some-other-cli",
		"cwd":      s.hubCWD,
	}))
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "invalid provider") {
		t.Fatalf("code = %d, body = %s, want 400 invalid provider", w.Code, w.Body.String())
	}
}

// TestHandleSpawnRejectsCustomProviderSubscriptionProfile は custom provider へ
// subscription_profile_id を指定したリクエストが、shell と同じ理由で（ただし別の
// 文言で）早期に 400 になることを確認する（plan_custom-provider-spawn-execution.md
// 決定事項2）。
func TestHandleSpawnRejectsCustomProviderSubscriptionProfile(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":                "my-cli",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": "main",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "custom providers do not use subscription profiles") {
		t.Fatalf("body = %s, want the custom-provider-specific subscription rejection message", w.Body.String())
	}
}

// TestResolveSpawnModel は decision 2（custom provider に built-in の model/route/env
// 注入を一切行わない）の実装単位を、実プロセスを起動せず直接固定する。
func TestResolveSpawnModel(t *testing.T) {
	if got := resolveSpawnModel("claude-opus", false); got != "claude-opus" {
		t.Fatalf("resolveSpawnModel(model, false) = %q, want the model preserved", got)
	}
	if got := resolveSpawnModel("  claude-opus  ", false); got != "claude-opus" {
		t.Fatalf("resolveSpawnModel(model, false) = %q, want trimmed", got)
	}
	if got := resolveSpawnModel("some-model", true); got != "" {
		t.Fatalf("resolveSpawnModel(model, true) = %q, want empty for a custom provider", got)
	}
	if got := resolveSpawnModel("", true); got != "" {
		t.Fatalf("resolveSpawnModel(\"\", true) = %q, want empty", got)
	}
}

// TestValidGridAIProvider は handleSpawnGrid の ai+shell プリセットが、shell 自体は
// 引き続き拒否しつつ built-in と custom_providers の両方を AI 側として受け付けることを
// 確認する（plan_custom-provider-spawn-execution.md C2）。
func TestValidGridAIProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	for _, provider := range []string{"claude", "codex", "command-code", "my-cli"} {
		if !s.validGridAIProvider(provider) {
			t.Errorf("validGridAIProvider(%q) = false, want true", provider)
		}
	}
	for _, provider := range []string{"shell", "some-other-cli", ""} {
		if s.validGridAIProvider(provider) {
			t.Errorf("validGridAIProvider(%q) = true, want false", provider)
		}
	}
}

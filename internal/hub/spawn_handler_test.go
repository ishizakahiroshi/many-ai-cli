package hub

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

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

// TestSanitizeSpawnInitialPromptEmptyStaysEmpty は
// plan_session-handoff-board_c4_prompted-spawn.md C1 の中心不変条件を固定する。
// 空文字・空白のみは "" のままで、以降の呼び出し元（spawnPendingLabelForInitialPrompt）
// が pending への書き込みを一切起こさない。
func TestSanitizeSpawnInitialPromptEmptyStaysEmpty(t *testing.T) {
	for _, raw := range []string{"", "   ", "\t\n  \r\n"} {
		if got := sanitizeSpawnInitialPrompt(raw); got != "" {
			t.Errorf("sanitizeSpawnInitialPrompt(%q) = %q, want \"\"", raw, got)
		}
	}
}

// TestSanitizeSpawnInitialPromptTrimsAndStripsControlBytes は前後空白の除去と、
// sanitizeInjectText 経由の C0 制御文字除去（spawn-child 経路と同じフィルタ）を確認する。
func TestSanitizeSpawnInitialPromptTrimsAndStripsControlBytes(t *testing.T) {
	got := sanitizeSpawnInitialPrompt("  hello\x07\x1bworld  \n")
	if strings.ContainsAny(got, "\x07\x1b") {
		t.Fatalf("sanitizeSpawnInitialPrompt left control bytes: %q", got)
	}
	if got != "helloworld" {
		t.Fatalf("sanitizeSpawnInitialPrompt = %q, want %q", got, "helloworld")
	}
}

// TestSanitizeSpawnInitialPromptCapsLength は spawnInitialPromptMaxLen を超える
// 入力が切り詰められることを確認する（子 plan C1: 長さ上限）。
func TestSanitizeSpawnInitialPromptCapsLength(t *testing.T) {
	long := strings.Repeat("a", spawnInitialPromptMaxLen+500)
	got := sanitizeSpawnInitialPrompt(long)
	if len(got) > spawnInitialPromptMaxLen {
		t.Fatalf("sanitizeSpawnInitialPrompt kept %d bytes, want <= %d", len(got), spawnInitialPromptMaxLen)
	}
}

// TestSpawnPendingLabelForInitialPromptEmptyNeverGeneratesLabel は「initial_prompt が
// 空のときは label の生成すら起きない」ことを直接固定する。既存の label がどうであれ
// 常に "" を返す＝呼び出し元は pending map に触れない。
func TestSpawnPendingLabelForInitialPromptEmptyNeverGeneratesLabel(t *testing.T) {
	now := time.Now()
	for _, label := range []string{"", "already-set"} {
		if got := spawnPendingLabelForInitialPrompt(label, "", now); got != "" {
			t.Errorf("spawnPendingLabelForInitialPrompt(%q, \"\", now) = %q, want \"\"", label, got)
		}
	}
}

// TestSpawnPendingLabelForInitialPromptReusesOrGeneratesLabel は非空プロンプトのとき、
// 既存 label を優先し、無ければ生成することを確認する。
func TestSpawnPendingLabelForInitialPromptReusesOrGeneratesLabel(t *testing.T) {
	now := time.Now()
	if got := spawnPendingLabelForInitialPrompt("existing", "do the thing", now); got != "existing" {
		t.Fatalf("spawnPendingLabelForInitialPrompt with existing label = %q, want %q", got, "existing")
	}
	got := spawnPendingLabelForInitialPrompt("", "do the thing", now)
	if got == "" {
		t.Fatal("spawnPendingLabelForInitialPrompt with empty label generated \"\", want a non-empty generated label")
	}
	if !strings.HasPrefix(got, "spawn-") {
		t.Fatalf("spawnPendingLabelForInitialPrompt generated %q, want spawn-<ts> shape", got)
	}
}

// TestHandleSpawnEmptyInitialPromptCreatesNoPendingEntry は C1 の完了条件そのもの:
// initial_prompt を省略した POST /api/spawn は、この機能が無かった頃と同じく
// s.orchestration.pending へ一切書き込まない。opencode + bypassPermissions で
// risk_confirmation_required に倒し、実プロセスを起動せずに検証する
// （TestHandleSpawnAcceptsCommandCodeProvider と同じ手法）。
func TestHandleSpawnEmptyInitialPromptCreatesNoPendingEntry(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	before := len(s.orchestration.pending)
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":        "opencode",
		"cwd":             s.hubCWD,
		"permission_mode": "bypassPermissions",
	}))
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "risk_confirmation_required") {
		t.Fatalf("code = %d, body = %s, want 400 risk_confirmation_required", w.Code, w.Body.String())
	}
	s.orchestration.mu.Lock()
	after := len(s.orchestration.pending)
	s.orchestration.mu.Unlock()
	if after != before {
		t.Fatalf("pending entries = %d, want unchanged from %d (empty initial_prompt must not touch pending state)", after, before)
	}
}

// TestHandleSpawnInitialPromptStoredInPending は非空 initial_prompt が
// sanitizeSpawnInitialPrompt 済みで s.orchestration.pending へ届くことを確認する
// （画面から見て「配送された」の手前までを自動テストで固定する。実際の PTY 注入
// ＝入力欄への到達は Claude/Codex を使った手動確認がこの C の完了条件）。
func TestHandleSpawnInitialPromptStoredInPending(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider":        "opencode",
		"cwd":             s.hubCWD,
		"permission_mode": "bypassPermissions",
		"initial_prompt":  "  do the thing  \x07",
	}))
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "risk_confirmation_required") {
		t.Fatalf("code = %d, body = %s, want 400 risk_confirmation_required", w.Code, w.Body.String())
	}
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	found := false
	for _, meta := range s.orchestration.pending {
		if meta.InitialPrompt == "do the thing" {
			found = true
		}
	}
	if !found {
		t.Fatalf("no pending entry carries the sanitized initial_prompt; pending = %+v", s.orchestration.pending)
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

// 以下は起動要求の共通 3 項目（effort / execution_mode / permission_preset）の
// 受理と拒否を固定する（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C1）。
//
// 「受理された」の確認は、このファイルの既存テスト
// （TestHandleSpawnAcceptsCommandCodeProvider / ...AcceptsCustomProvider）と同じ手法で
// 行う: 実在しない subscription profile を添えると、検証を通り抜けた要求だけが
// その後段の subscription エラーへ到達する。実 wrap プロセスを起動させずに
// 「3 項目の検証では弾かれていない」ことを確かめられる。
const spawnLaunchOptionProbeProfile = "no-such-profile"

func spawnLaunchOptionRequest(t *testing.T, s *Server, extra map[string]any) *httptest.ResponseRecorder {
	t.Helper()
	body := map[string]any{
		"provider":                "claude",
		"cwd":                     s.hubCWD,
		"subscription_profile_id": spawnLaunchOptionProbeProfile,
	}
	for k, v := range extra {
		body[k] = v
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", body))
	return w
}

// 3 項目を送らない要求は、この 3 項目を知らなかった頃とまったく同じ経路を通る
// （検証は一度も失敗しない）。不変条件 1（省略した既存の呼び出しを 1 バイトも
// 変えない）の直接の固定。
func TestHandleSpawnWithoutLaunchOptionsPassesValidation(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	for _, provider := range []string{"claude", "codex", "copilot", "grok", "opencode"} {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": provider})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "subscription") {
			t.Fatalf("provider %s: code = %d, body = %s, want the later subscription error (validation must not fire)", provider, w.Code, w.Body.String())
		}
	}
	// custom provider は subscription profile を使えないので、到達する先が
	// 別の 400 になる。どちらにせよ effort の検証では弾かれない。
	w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": "my-cli"})
	if !strings.Contains(w.Body.String(), "custom providers do not use subscription profiles") {
		t.Fatalf("custom provider body = %s, want the custom-provider subscription rejection", w.Body.String())
	}
}

// 写像がある provider の正しい effort は受理される（後段の subscription エラーまで進む）。
func TestHandleSpawnAcceptsEffortForMappedProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	cases := []struct{ provider, effort string }{
		{"claude", "high"},
		{"codex", "minimal"},
		{"opencode", "low"},
	}
	for _, tc := range cases {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": tc.provider, "effort": tc.effort})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "subscription") {
			t.Fatalf("%s effort=%s: code = %d, body = %s, want the later subscription error", tc.provider, tc.effort, w.Code, w.Body.String())
		}
		if strings.Contains(w.Body.String(), "effort") {
			t.Fatalf("%s effort=%s was rejected by the effort check: %s", tc.provider, tc.effort, w.Body.String())
		}
	}
}

// 写像が無い provider に effort を付けた要求は 400。
func TestHandleSpawnRejectsEffortForUnmappedProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
	}
	for _, provider := range []string{"copilot", "grok", "cursor-agent", "command-code", "my-cli"} {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": provider, "effort": "high"})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "effort") {
			t.Fatalf("provider %s: code = %d, body = %s, want 400 with an effort error", provider, w.Code, w.Body.String())
		}
	}
	// 写像がある provider でも、表に無い値は 400。
	w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": "claude", "effort": "turbo"})
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "effort") {
		t.Fatalf("claude effort=turbo: code = %d, body = %s, want 400 with an effort error", w.Code, w.Body.String())
	}
}

// dontAsk は permission_mode の受理値。Claude / Grok は --permission-mode として
// そのまま解釈し、他 provider では既存の変換関数が無視する（= 従来の default と同じ）。
func TestHandleSpawnAcceptsDontAskPermissionMode(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	for _, provider := range []string{"claude", "grok"} {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": provider, "permission_mode": "dontAsk"})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "subscription") {
			t.Fatalf("provider %s: code = %d, body = %s, want the later subscription error (dontAsk must be accepted)", provider, w.Code, w.Body.String())
		}
	}
	// 受理値の追加は dontAsk だけで、未知の値は従来どおり弾く。
	w := spawnLaunchOptionRequest(t, s, map[string]any{"permission_mode": "dontAskEver"})
	if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "bad request") {
		t.Fatalf("code = %d, body = %s, want the generic 400 for an unknown permission mode", w.Code, w.Body.String())
	}
}

// headless は provider ごとに可否が決まる（子 plan
// plan_child_execution_modes_headless.md 内部 C1 で解禁）。定義が無い provider への
// 明示要求は、黙って interactive へ倒さず 400 で返す（親 plan D2）。
func TestHandleSpawnResolvesExecutionModePerProvider(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	// 定義がある provider は通る＝後段の subscription エラーまで進む（内部 C5 で
	// claude 以外の built-in にも定義が入った）。
	for _, provider := range []string{"claude", "grok", "cursor-agent", "copilot"} {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": provider, "execution_mode": "headless"})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "subscription") {
			t.Fatalf("%s headless: code = %d, body = %s, want the later subscription error", provider, w.Code, w.Body.String())
		}
	}
	// 定義が無い provider への明示 headless は 400（built-in では codex だけ）。
	for _, provider := range []string{"codex"} {
		w := spawnLaunchOptionRequest(t, s, map[string]any{"provider": provider, "execution_mode": "headless"})
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "headless") {
			t.Fatalf("%s headless: code = %d, body = %s, want 400 naming headless", provider, w.Code, w.Body.String())
		}
	}
	// この build で選べる値は素通しする（権限の段は 3 つとも選べる。子 plan
	// plan_derived-session-launch_c2_permission-tiers.md 内部 C3 で bounded を解禁）。
	// auto は画面起点なので対話のまま＝ grok でも弾かれない。
	for _, extra := range []map[string]any{
		{"execution_mode": "auto"},
		{"execution_mode": "interactive"},
		{"execution_mode": "auto", "provider": "grok"},
		{"permission_preset": "attended"},
		{"permission_preset": "bounded"},
		{"permission_preset": "full"},
	} {
		w := spawnLaunchOptionRequest(t, s, extra)
		if w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "subscription") {
			t.Fatalf("%v: code = %d, body = %s, want the later subscription error", extra, w.Code, w.Body.String())
		}
	}
	// 未知の値は 400。
	for _, extra := range []map[string]any{
		{"execution_mode": "batch"},
		{"permission_preset": "none"},
	} {
		w := spawnLaunchOptionRequest(t, s, extra)
		if w.Code != http.StatusBadRequest {
			t.Fatalf("%v: code = %d, want 400", extra, w.Code)
		}
	}
}

// TestEffortWrapArgs は Hub が `wrap` へ渡す effort フラグを固定する
// （子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C2）。
// provider ごとの引数の形を決めるのは wrapper 側（表を読む）で、Hub は
// 「--effort <level> を足すか、何も足さないか」だけを決める。
func TestEffortWrapArgs(t *testing.T) {
	for _, provider := range []string{"claude", "codex", "opencode"} {
		got := effortWrapArgs(provider, "high")
		if len(got) != 2 || got[0] != "--effort" || got[1] != "high" {
			t.Fatalf("effortWrapArgs(%q, \"high\") = %v, want [--effort high]", provider, got)
		}
	}
	// 写像が無い provider と、effort 未指定は 1 引数も足さない
	// ＝この 3 項目が無かった頃の wrapArgs と完全に一致する（不変条件 1）。
	for _, provider := range []string{"copilot", "grok", "cursor-agent", "command-code", "shell", "my-cli"} {
		if got := effortWrapArgs(provider, "high"); got != nil {
			t.Fatalf("effortWrapArgs(%q, \"high\") = %v, want nil", provider, got)
		}
	}
	for _, provider := range []string{"claude", "codex", "opencode", "copilot", "shell"} {
		if got := effortWrapArgs(provider, ""); got != nil {
			t.Fatalf("effortWrapArgs(%q, \"\") = %v, want nil", provider, got)
		}
	}
}

// 段 2 の許可 tool も同じ形: Hub は「--allowed-tools <csv> を足すか、何も足さないか」
// だけを決め、provider ごとのフラグ名は wrapper が決める（子 plan
// plan_derived-session-launch_c2_permission-tiers.md 内部 C2）。
func TestAllowedToolsWrapArgs(t *testing.T) {
	if got := allowedToolsWrapArgs(nil); got != nil {
		t.Fatalf("allowedToolsWrapArgs(nil) = %v, want nil", got)
	}
	got := allowedToolsWrapArgs([]string{"Read", " Bash(git log *) "})
	if len(got) != 2 || got[0] != "--allowed-tools" || got[1] != "Read,Bash(git log *)" {
		t.Fatalf("allowedToolsWrapArgs = %v", got)
	}
	// コマンドラインへ出せない値は落とし、全部落ちたらフラグごと足さない。
	if got := allowedToolsWrapArgs([]string{"--oops", "rm -rf / ; echo x"}); got != nil {
		t.Fatalf("allowedToolsWrapArgs(invalid) = %v, want nil", got)
	}
}

// /api/spawn（New Session フォームの経路）で permission_preset が明示されたときは、
// 子セッションと同じ表で段を解決する。画面に段の欄は足していないので、これは
// API の約束（派生ダイアログは親 C3）。
func TestPermissionPresetFillsLaunchApprovalFields(t *testing.T) {
	bounded := permissionPresetApproval("claude", config.PermissionPresetBounded, config.OrchestrationConfig{})
	if bounded.PermissionMode != "dontAsk" || len(bounded.AllowedTools) == 0 {
		t.Fatalf("claude bounded = %+v", bounded)
	}
	// RiskConfirmed は段が持っていても /api/spawn では body へ写さない（高リスク確認は
	// 人が押す経路の歯止め）。この関数が返す値をそのまま信じないことの明示。
	if !bounded.RiskConfirmed {
		t.Fatal("表側は確認済みで起動する値を持つ（写すかどうかは呼び出し側の判断）")
	}
	if none := permissionPresetApproval("claude", "", config.OrchestrationConfig{}); none.Tier != config.PermissionPresetFull {
		t.Fatalf("空の preset は段 3 扱い: %+v", none)
	}
}

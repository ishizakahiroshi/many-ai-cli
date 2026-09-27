package hub

import (
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

// 表の形そのものを固定する。段 1 と段 3 は全 provider にあり、段 2 は
// 「聞かずに範囲を限る設定を持つ CLI」にだけある。段 2 の有無は確認ダイアログの
// 注意文（全許可へ落ちる旨）の出し分けそのものなので、行を足すときは
// ここも一緒に読む。
func TestChildPermissionTableHasEveryTier(t *testing.T) {
	boundedProviders := map[string]bool{
		"claude": true, "codex": true, "copilot": true, "opencode": true,
		// shell は AI を走らせないので承認の概念が無い。3 段とも「何も足さない」。
		"shell": true,
	}
	seen := map[string]bool{}
	for _, row := range childPermissionTable {
		if row.Provider == "" {
			t.Fatalf("provider 名の無い行がある: %+v", row)
		}
		if seen[row.Provider] {
			t.Fatalf("provider %q の行が重複している", row.Provider)
		}
		seen[row.Provider] = true
		// 段 1 は「Hub が何も足さない」なので空であることが正しい姿。
		if row.Attended.PermissionMode != "" || row.Attended.Sandbox != "" || row.Attended.AskForApproval != "" || row.Attended.RiskConfirmed {
			t.Fatalf("provider %q: 段 1 は何も足さないはず: %+v", row.Provider, row.Attended)
		}
		if (row.Bounded != nil) != boundedProviders[row.Provider] {
			t.Fatalf("provider %q: 段 2 の有無が表と食い違う (bounded=%v)", row.Provider, row.Bounded != nil)
		}
	}
	for _, provider := range orchestrationProviders {
		if !seen[provider] {
			t.Fatalf("orchestration の provider %q が表に無い", provider)
		}
	}
}

// 画面から人が起動した子（親 C3 の origin: "ui"）は段 1。承認は Hub の承認パネルへ
// 来て人が答える、が有人の意味なので、Hub は何も足さない。
func TestResolveChildPermissionUIOriginIsAttended(t *testing.T) {
	got := resolveChildPermission("claude", "", config.ExecutionModeInteractive, launchOriginUI, config.OrchestrationConfig{})
	if got.Tier != config.PermissionPresetAttended {
		t.Fatalf("tier = %q, want attended", got.Tier)
	}
	if got.PermissionMode != "" || got.Sandbox != "" || got.AskForApproval != "" || got.RiskConfirmed {
		t.Fatalf("有人の子に権限を足してはいけない: %+v", got)
	}
}

// headless は人が押しても無人。誰も承認プロンプトに答えられないので段 1 にしない。
func TestResolveChildPermissionHeadlessUIIsNotAttended(t *testing.T) {
	got := resolveChildPermission("claude", "", config.ExecutionModeHeadless, launchOriginUI, config.OrchestrationConfig{})
	if got.Tier != config.PermissionPresetFull {
		t.Fatalf("tier = %q, want full (既定未設定なので段 3)", got.Tier)
	}
}

// conductor（AI の orchestrate spawn）と relay は無人。既定未設定なら今日どおり段 3。
func TestResolveChildPermissionConductorDefaultsToConfiguredTier(t *testing.T) {
	full := resolveChildPermission("claude", "", config.ExecutionModeInteractive, launchOriginConductor, config.OrchestrationConfig{})
	if full.Tier != config.PermissionPresetFull || full.PermissionMode != "bypassPermissions" || !full.RiskConfirmed {
		t.Fatalf("既定未設定 = %+v, want 段 3", full)
	}

	bounded := resolveChildPermission("claude", "", config.ExecutionModeInteractive, launchOriginConductor,
		config.OrchestrationConfig{ChildPermissionDefault: config.PermissionPresetBounded})
	if bounded.Tier != config.PermissionPresetBounded || bounded.PermissionMode != "dontAsk" {
		t.Fatalf("child_permission_default: bounded = %+v", bounded)
	}
	if len(bounded.AllowedTools) == 0 {
		t.Fatal("段 2 の claude は許可 tool の一覧を持つはず")
	}
}

// 段 2 が無い provider に bounded を指定したら段 3 へ落ちる。**落ちたことを黙らせない**
// （親 plan 不変条件 5）。FallbackFrom がダイアログの注意文の根拠になる。
func TestResolveChildPermissionBoundedFallsBackAndSaysSo(t *testing.T) {
	for _, provider := range []string{"grok", "cursor-agent", "command-code"} {
		got := resolveChildPermission(provider, config.PermissionPresetBounded, "", launchOriginConductor, config.OrchestrationConfig{})
		if got.Tier != config.PermissionPresetFull {
			t.Fatalf("%s: tier = %q, want full", provider, got.Tier)
		}
		if got.FallbackFrom != config.PermissionPresetBounded {
			t.Fatalf("%s: 段 2 が無いことが伝わらない: %+v", provider, got)
		}
		if got.PermissionMode != "bypassPermissions" {
			t.Fatalf("%s: 落ちた先が段 3 になっていない: %+v", provider, got)
		}
	}
	claude := resolveChildPermission("claude", config.PermissionPresetBounded, "", launchOriginConductor, config.OrchestrationConfig{})
	if claude.FallbackFrom != "" {
		t.Fatalf("段 2 がある provider で fallback を立ててはいけない: %+v", claude)
	}
}

// 段 2 の実効値。ここが変わると無人の子が触れる範囲が変わるので、値そのものを固定する。
func TestResolveChildPermissionBoundedValuesPerProvider(t *testing.T) {
	cfg := config.OrchestrationConfig{}
	codex := resolveChildPermission("codex", config.PermissionPresetBounded, "", launchOriginConductor, cfg)
	if codex.Sandbox != "workspace-write" || codex.AskForApproval != "never" {
		t.Fatalf("codex 段 2 = %+v", codex)
	}
	if !codex.RiskConfirmed {
		t.Fatal("codex 段 2 は ask-for-approval never を含むので高リスク確認済みで起動する必要がある")
	}
	for _, provider := range []string{"copilot", "opencode"} {
		got := resolveChildPermission(provider, config.PermissionPresetBounded, "", launchOriginConductor, cfg)
		if got.PermissionMode != config.PermissionModeBounded {
			t.Fatalf("%s 段 2 = %+v, want 内部マーカー", provider, got)
		}
	}
	copilot := resolveChildPermission("copilot", config.PermissionPresetBounded, "", launchOriginConductor, cfg)
	if len(copilot.AllowedTools) == 0 {
		t.Fatal("copilot 段 2 は --allow-tool の一覧を持つはず")
	}
}

// 段 2 の内蔵 allowlist は「読む・直す・検証する・記録する」まで。取り返しのつかない
// 操作は入れない（入っていないので blocked になり、子がそう報告する方が正しい）。
func TestBoundedAllowedToolsExcludeIrreversibleCommands(t *testing.T) {
	for _, provider := range []string{"claude", "copilot"} {
		joined := strings.Join(config.OrchestrationConfig{}.BoundedAllowedToolsFor(provider), " ")
		if joined == "" {
			t.Fatalf("%s の内蔵 allowlist が空", provider)
		}
		for _, forbidden := range []string{"git push", "git reset", "git clean", "rm "} {
			if strings.Contains(joined, forbidden) {
				t.Fatalf("%s の allowlist に %q が入っている: %s", provider, forbidden, joined)
			}
		}
		if !strings.Contains(joined, "git commit") {
			t.Fatalf("%s の allowlist に git commit が無い: %s", provider, joined)
		}
	}
}

// child_full_bypass: false は「既定を埋めない」の意味を変えない。段の指定という
// 近道でも埋まらない（見送り台帳 D-12 が守っている安全弁を preset で空にしない）。
func TestResolveChildPermissionBypassOffFillsNothing(t *testing.T) {
	off := false
	cfg := config.OrchestrationConfig{ChildFullBypass: &off, ChildPermissionDefault: config.PermissionPresetBounded}
	for _, preset := range []string{"", config.PermissionPresetAttended, config.PermissionPresetBounded, config.PermissionPresetFull} {
		got := resolveChildPermission("claude", preset, "", launchOriginConductor, cfg)
		if got.PermissionMode != "" || got.Sandbox != "" || got.AskForApproval != "" || got.RiskConfirmed || len(got.AllowedTools) > 0 {
			t.Fatalf("preset %q: bypass off なのに埋まっている: %+v", preset, got)
		}
	}
}

// 呼び出し側の明示値は段より強い。3 項目を知らない既存の呼び出しが 1 バイトも
// 変わらないのは、この「空欄だけ埋める」規律そのもの。
func TestApplyChildPermissionKeepsExplicitValues(t *testing.T) {
	body := &spawnChildRequest{Provider: "codex", PermissionPreset: config.PermissionPresetBounded, Sandbox: "read-only"}
	applyChildPermission(body, config.OrchestrationConfig{})
	if body.Sandbox != "read-only" {
		t.Fatalf("明示値が上書きされた: %+v", body)
	}
	if body.AskForApproval != "never" {
		t.Fatalf("空欄は段から埋まるはず: %+v", body)
	}
}

// 実行モードの解決（子 plan plan_child_execution_modes_headless.md 内部 C1）。
// 純関数の表は internal/config 側にあるので、ここで固定するのは Hub が足している
// 2 つ — provider ごとの可否と、要求がモードを省略したときの config 既定。
func TestApplyChildExecutionMode(t *testing.T) {
	cfg := &config.Config{}
	// 不変条件 1: 省略した要求は空のまま。"interactive" へ書き換えもしない。
	body := &spawnChildRequest{Provider: "claude"}
	if err := applyChildExecutionMode(body, cfg); err != nil {
		t.Fatalf("省略した要求で失敗してはいけない: %v", err)
	}
	if body.ExecutionMode != "" {
		t.Fatalf("execution_mode = %q, want 空のまま", body.ExecutionMode)
	}

	// auto × 無人（conductor）× 定義あり → headless。
	body = &spawnChildRequest{Provider: "claude", ExecutionMode: config.ExecutionModeAuto}
	if err := applyChildExecutionMode(body, cfg); err != nil {
		t.Fatalf("auto: %v", err)
	}
	if body.ExecutionMode != config.ExecutionModeHeadless {
		t.Fatalf("execution_mode = %q, want headless", body.ExecutionMode)
	}

	// auto × 定義なし → 対話へ倒す（auto だけが倒してよい）。定義が無い built-in は
	// 今は codex だけ（理由は internal/config/headless.go の表のコメント）。
	body = &spawnChildRequest{Provider: "codex", ExecutionMode: config.ExecutionModeAuto}
	if err := applyChildExecutionMode(body, cfg); err != nil {
		t.Fatalf("auto (定義なし): %v", err)
	}
	if body.ExecutionMode != config.ExecutionModeInteractive {
		t.Fatalf("execution_mode = %q, want interactive", body.ExecutionMode)
	}

	// auto × 画面起点 → 対話。人が開いたものは人が打てる方が価値がある。
	body = &spawnChildRequest{Provider: "claude", ExecutionMode: config.ExecutionModeAuto, Origin: launchOriginUI}
	if err := applyChildExecutionMode(body, cfg); err != nil {
		t.Fatalf("auto (ui): %v", err)
	}
	if body.ExecutionMode != config.ExecutionModeInteractive {
		t.Fatalf("execution_mode = %q, want interactive", body.ExecutionMode)
	}

	// 明示 headless × 定義なし → エラー（黙って対話へ倒さない・親 plan D2）。
	body = &spawnChildRequest{Provider: "codex", ExecutionMode: config.ExecutionModeHeadless}
	if err := applyChildExecutionMode(body, cfg); err == nil {
		t.Fatal("定義が無い provider への明示 headless はエラーにする")
	}

	// config の既定は「要求がモードを省略したとき」だけに効く。
	withDefault := &config.Config{}
	withDefault.Orchestration.ChildExecutionMode = config.ExecutionModeAuto
	body = &spawnChildRequest{Provider: "claude"}
	if err := applyChildExecutionMode(body, withDefault); err != nil {
		t.Fatalf("config 既定: %v", err)
	}
	if body.ExecutionMode != config.ExecutionModeHeadless {
		t.Fatalf("execution_mode = %q, want headless (child_execution_mode: auto)", body.ExecutionMode)
	}
	body = &spawnChildRequest{Provider: "claude", ExecutionMode: config.ExecutionModeInteractive}
	if err := applyChildExecutionMode(body, withDefault); err != nil {
		t.Fatalf("明示 interactive: %v", err)
	}
	if body.ExecutionMode != config.ExecutionModeInteractive {
		t.Fatalf("execution_mode = %q, want 明示値が既定に勝つ", body.ExecutionMode)
	}
}

// config.yaml の custom_providers に headless 定義があれば、その CLI も無人の子に
// なれる（親 plan D9: provider 差分は定義で持つ）。Hub 側に provider の switch は
// 増えない。
func TestHeadlessCapableProviderReadsCustomProviders(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{
		{ID: "my-cli", Command: "my-cli", Headless: &config.HeadlessDef{
			Args: []string{"--print"}, Format: config.HeadlessFormatText,
		}},
		{ID: "plain-cli", Command: "plain-cli"},
	}}
	if !headlessCapableProvider("my-cli", cfg) {
		t.Error("headless 定義を書いた custom provider は capable のはず")
	}
	if headlessCapableProvider("plain-cli", cfg) {
		t.Error("定義が無い custom provider は capable ではない")
	}
	if !headlessCapableProvider("claude", nil) {
		t.Error("built-in の定義は cfg 無しでも引けるはず")
	}
}

// 実行モードの解決は権限の段より先。headless に解決した子は、画面から人が押した
// ものでも段 1（有人）にならない。この 2 つの順序が入れ替わると、確認ダイアログが
// 見せた段と実際の段がずれる。
func TestExecutionModeResolutionRunsBeforeTiers(t *testing.T) {
	body := &spawnChildRequest{Provider: "claude", ExecutionMode: config.ExecutionModeHeadless, Origin: launchOriginUI}
	if err := applyChildExecutionMode(body, &config.Config{}); err != nil {
		t.Fatalf("claude headless: %v", err)
	}
	applyChildPermission(body, config.OrchestrationConfig{})
	if body.PermissionMode != "bypassPermissions" {
		t.Fatalf("permission_mode = %q, want 無人の既定（段 3）", body.PermissionMode)
	}

	// 同じ画面起点でも auto は対話へ解決するので、段 1 のまま（何も足さない）。
	auto := &spawnChildRequest{Provider: "claude", ExecutionMode: config.ExecutionModeAuto, Origin: launchOriginUI}
	if err := applyChildExecutionMode(auto, &config.Config{}); err != nil {
		t.Fatalf("claude auto: %v", err)
	}
	applyChildPermission(auto, config.OrchestrationConfig{})
	if auto.PermissionMode != "" || auto.RiskConfirmed {
		t.Fatalf("画面から人が開く対話の子に権限を足してはいけない: %+v", auto)
	}
}

// 未知の provider（custom provider・ダイアログが残す選択肢）は表に無い。
// 表に無い＝段 2 は作れない（任意の CLI のフラグを many-ai-cli は知らない）ので、
// 今日と同じ段 3 の既定へ倒れる。
func TestResolveChildPermissionUnknownProviderUsesFallbackRow(t *testing.T) {
	got := resolveChildPermission("made-up-cli", "", "", launchOriginConductor, config.OrchestrationConfig{})
	if got.PermissionMode != "bypassPermissions" || !got.RiskConfirmed {
		t.Fatalf("未知 provider = %+v", got)
	}
	bounded := resolveChildPermission("made-up-cli", config.PermissionPresetBounded, "", launchOriginConductor, config.OrchestrationConfig{})
	if bounded.FallbackFrom != config.PermissionPresetBounded {
		t.Fatalf("未知 provider の bounded は fallback するはず: %+v", bounded)
	}
}

// 承認者がダイアログで段 2 を選んだら、その子は dontAsk + allowlist で起動する。
// 決定の適用（applySpawnConfirmationDecision）→ 検証 → 段の解決、という
// performSpawnWithAdmission と同じ順序で通す。承認後に段をもう一度解決し直す経路は
// 無い（最終の body を 1 度だけ見る）ことも、この並びが示している。
func TestApproverSelectedBoundedStartsTheChildWithDontAsk(t *testing.T) {
	launch := applySpawnConfirmationDecision(
		spawnChildRequest{Role: "review", Provider: "claude"},
		spawnConfirmationDecision{PermissionPreset: launchOptionPtr(config.PermissionPresetBounded)},
	)
	if err := validateLaunchRequestOptions(launch.Provider, &launch.Effort, &launch.ExecutionMode, &launch.PermissionPreset); err != nil {
		t.Fatalf("承認者が選べる段は検証を通るはず: %v", err)
	}
	applyChildPermission(&launch, config.OrchestrationConfig{})
	if launch.PermissionMode != "dontAsk" {
		t.Fatalf("permission_mode = %q, want dontAsk", launch.PermissionMode)
	}
	if len(launch.AllowedTools) == 0 {
		t.Fatal("段 2 の claude は allowlist 付きで起動するはず")
	}
	if !launch.RiskConfirmed {
		t.Fatal("モデル変更で起動が拒否されないよう、段 2 も確認済みで起動する")
	}
}

// config.yaml の bounded_allowed_tools は provider ごとに内蔵既定を置き換える。
// コマンドラインへ出せない形の値は落とし、落としたことは Warnings が伝える。
func TestBoundedAllowedToolsOverrideAndFilter(t *testing.T) {
	cfg := config.OrchestrationConfig{BoundedAllowedTools: map[string][]string{
		"claude": {"Read", "Bash(mytool --check)", "--oops", "rm -rf / ; echo x"},
	}}
	got := cfg.BoundedAllowedToolsFor("claude")
	want := []string{"Read", "Bash(mytool --check)"}
	if len(got) != len(want) {
		t.Fatalf("BoundedAllowedToolsFor = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("BoundedAllowedToolsFor = %v, want %v", got, want)
		}
	}
	if list := cfg.BoundedAllowedToolsFor("copilot"); len(list) == 0 {
		t.Fatal("上書きしていない provider は内蔵既定のまま残るはず")
	}
}

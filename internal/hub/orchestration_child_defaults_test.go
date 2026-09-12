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

	// 親が custom provider のときも同じ受け皿へ（plan_custom-provider-extension-triage.md C1）。
	// customProviderSession 側の分岐ではなく validOrchestrationProvider が built-in
	// 固定リストのため常に偽になる経路で codex へ落ちることを固定する。
	s.cfg.UserPrefs.Spawn.RoleProvider = nil
	if got := s.resolveChildProviderFallback(&session{Provider: "my-cli", customProviderSession: true}, "review"); got != "codex" {
		t.Fatalf("親が custom provider のときの受け皿が違う: got %q", got)
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

	if got := s.resolveChildSubscription(parent, "claude", "review"); got != "claude-work" {
		t.Fatalf("同じ provider なら親の profile を引き継ぐはず: got %q", got)
	}
	if got := s.resolveChildSubscription(parent, "codex", "review"); got != "codex-personal" {
		t.Fatalf("別 provider ではパネルの記憶を使うはず: got %q", got)
	}
	if got := s.resolveChildSubscription(parent, "grok", "review"); got != "" {
		t.Fatalf("記憶が無い provider は空（CLI 既定ログイン）のはず: got %q", got)
	}
	// 親が profile を持たないときは、同じ provider でもパネルの記憶へ落ちる。
	if got := s.resolveChildSubscription(&session{Provider: "claude"}, "claude", "review"); got != "claude-personal" {
		t.Fatalf("親に profile が無いときの解決が違う: got %q", got)
	}
}

func TestResolveChildSubscriptionRoleAssignmentWins(t *testing.T) {
	s := newTestServer()
	parent := &session{
		Provider:              "claude",
		SubscriptionProfileID: "claude-parent",
		OrchestrationID:       "o1",
	}
	s.cfg.UserPrefs.Spawn.Defaults = map[string]string{
		"subscription_claude": "claude-default",
	}
	s.orchestration.roles = map[string]map[string]orchestrationRoleAssignment{
		"o1": {
			"review": {Provider: "claude", Subscription: "claude-review"},
		},
	}

	if got := s.resolveChildSubscription(parent, "claude", "review"); got != "claude-review" {
		t.Fatalf("role assignment の profile が最優先になるはず: got %q", got)
	}
}

func TestResolveChildSubscriptionEmptyRoleAssignmentFallsBack(t *testing.T) {
	s := newTestServer()
	s.cfg.UserPrefs.Spawn.Defaults = map[string]string{
		"subscription_claude": "claude-default",
	}
	s.orchestration.roles = map[string]map[string]orchestrationRoleAssignment{
		"o1": {
			"review": {Provider: "claude", Subscription: "  "},
		},
	}

	parent := &session{
		Provider:              "claude",
		SubscriptionProfileID: "claude-parent",
		OrchestrationID:       "o1",
	}
	if got := s.resolveChildSubscription(parent, "claude", "review"); got != "claude-parent" {
		t.Fatalf("空のrole profileでは親を引き継ぐはず: got %q", got)
	}
	if got := s.resolveChildSubscription(&session{Provider: "claude", OrchestrationID: "o1"}, "claude", "review"); got != "claude-default" {
		t.Fatalf("親profileが無いときは既定へ落ちるはず: got %q", got)
	}
}

func TestResolveChildSubscriptionMissingRoleMapFallsBack(t *testing.T) {
	s := newTestServer()
	s.cfg.UserPrefs.Spawn.Defaults = map[string]string{
		"subscription_claude": "claude-default",
	}

	if got := s.resolveChildSubscription(nil, "claude", "review"); got != "claude-default" {
		t.Fatalf("親がnilでも既定へ落ちるはず: got %q", got)
	}
	parent := &session{
		Provider:              "claude",
		SubscriptionProfileID: "claude-parent",
		OrchestrationID:       "missing",
	}
	if got := s.resolveChildSubscription(parent, "claude", "review"); got != "claude-parent" {
		t.Fatalf("role mapが無くても親を引き継ぐはず: got %q", got)
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

// C2 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): 症状A（--provider 無視）の
// 修正。明示 requested は roleAssigned / remembered / parentProvider のどれにも負けない。
func TestResolveChildProviderExplicitAlwaysWins(t *testing.T) {
	provider, source := resolveChildProvider("claude", "codex", "grok", "cursor-agent")
	if provider != "claude" || source != providerSourceExplicit {
		t.Fatalf("explicit requested が勝っていない: provider=%q source=%q", provider, source)
	}
	// role map / remembered / parent が全部揃っていても、明示指定が唯一の decisive factor。
	provider, source = resolveChildProvider("codex", "claude", "claude", "claude")
	if provider != "codex" || source != providerSourceExplicit {
		t.Fatalf("explicit requested が勝っていない(2): provider=%q source=%q", provider, source)
	}
}

// requested が空のときの解決順: role_map → remembered → parent → default(codex)。
// 無効な remembered / parentProvider（旧版の値・custom provider 等）はスキップされる。
func TestResolveChildProviderFallsThroughInOrder(t *testing.T) {
	if provider, source := resolveChildProvider("", "claude", "grok", "cursor-agent"); provider != "claude" || source != providerSourceRoleMap {
		t.Fatalf("role_map が優先されていない: provider=%q source=%q", provider, source)
	}
	if provider, source := resolveChildProvider("", "", "grok", "cursor-agent"); provider != "grok" || source != providerSourceRemembered {
		t.Fatalf("remembered が優先されていない: provider=%q source=%q", provider, source)
	}
	if provider, source := resolveChildProvider("", "", "", "cursor-agent"); provider != "cursor-agent" || source != providerSourceParent {
		t.Fatalf("parent が使われていない: provider=%q source=%q", provider, source)
	}
	if provider, source := resolveChildProvider("", "", "", ""); provider != "codex" || source != providerSourceDefault {
		t.Fatalf("既定への受け皿が違う: provider=%q source=%q", provider, source)
	}
	// 無効な remembered（旧版の壊れた値）は無視して親へ落ちる。
	if provider, source := resolveChildProvider("", "", "ChatGPT", "cursor-agent"); provider != "cursor-agent" || source != providerSourceParent {
		t.Fatalf("無効な remembered を採用している: provider=%q source=%q", provider, source)
	}
	// 無効な parentProvider（shell / custom provider 等）は無視して default へ落ちる。
	if provider, source := resolveChildProvider("", "", "", "shell"); provider != "codex" || source != providerSourceDefault {
		t.Fatalf("無効な parentProvider を採用している: provider=%q source=%q", provider, source)
	}
}

// resolveSpawnChildProvider は handleSpawnChild の役割対応表・記憶・親 provider を束ねた
// Server 側の入口。明示 --provider が role map（起動時に設定した対応表）にも記憶にも
// 負けないことを、実際の呼び出し経路に近い形で固定する（mer の role_provider:
// mer-c5-rotation: codex のような既存の記憶があっても、明示指定なら claude で起動する）。
func TestResolveSpawnChildProviderExplicitBeatsRoleMapAndMemory(t *testing.T) {
	s := newTestServer()
	parent := &session{Provider: "cursor-agent"}
	s.cfg.UserPrefs.Spawn.RoleProvider = map[string]string{"review": "codex"}
	s.orchestration.roles = map[string]map[string]orchestrationRoleAssignment{
		"o1": {"review": {Provider: "codex", Model: "gpt-5"}},
	}
	parent.OrchestrationID = "o1"

	body := &spawnChildRequest{Role: "review", Provider: "claude"}
	source := s.resolveSpawnChildProvider(parent, body)
	if body.Provider != "claude" || source != providerSourceExplicit {
		t.Fatalf("明示 provider が負けている: body.Provider=%q source=%q", body.Provider, source)
	}
	// 明示 --provider を渡しても、role map の model 補完は生きている（model は未指定のまま）。
	if body.Model != "gpt-5" {
		t.Fatalf("role map の model 補完が効いていない: body.Model=%q", body.Model)
	}

	// provider を省略すると role map → remembered → parent の順で解決される。
	body2 := &spawnChildRequest{Role: "review"}
	source2 := s.resolveSpawnChildProvider(parent, body2)
	if body2.Provider != "codex" || source2 != providerSourceRoleMap {
		t.Fatalf("role map が使われていない: body.Provider=%q source=%q", body2.Provider, source2)
	}

	// role map に無い役割は記憶 → 親の順。
	body3 := &spawnChildRequest{Role: "implementation"}
	source3 := s.resolveSpawnChildProvider(parent, body3)
	if body3.Provider != "cursor-agent" || source3 != providerSourceParent {
		t.Fatalf("記憶が無い役割は親に落ちるはず: body.Provider=%q source=%q", body3.Provider, source3)
	}
}

// C2 item 2: 承認ダイアログの決定が確認前の値と違うときだけ board 用の1行を返す。
// 同じなら空文字（＝書かない）。
func TestProviderConfirmationChangeNote(t *testing.T) {
	if note := providerConfirmationChangeNote("claude", "claude"); note != "" {
		t.Fatalf("一致するときは空のはず: %q", note)
	}
	note := providerConfirmationChangeNote("claude", "codex")
	if !strings.Contains(note, "requested=claude") || !strings.Contains(note, "decided=codex") {
		t.Fatalf("note の内容が不足: %q", note)
	}
}

func TestApplyChildApprovalDefaultsFullBypass(t *testing.T) {
	claude := &spawnChildRequest{Provider: "claude"}
	applyChildApprovalDefaults(claude, true)
	if claude.PermissionMode != "bypassPermissions" || !claude.RiskConfirmed {
		t.Fatalf("claude defaults = %+v", claude)
	}

	codex := &spawnChildRequest{Provider: "codex"}
	applyChildApprovalDefaults(codex, true)
	if codex.AskForApproval != "never" || codex.Sandbox != "danger-full-access" || !codex.RiskConfirmed {
		t.Fatalf("codex defaults = %+v", codex)
	}

	shell := &spawnChildRequest{Provider: "shell"}
	applyChildApprovalDefaults(shell, true)
	if shell.PermissionMode != "" || shell.RiskConfirmed {
		t.Fatalf("shell should stay untouched: %+v", shell)
	}
}

func TestApplyChildApprovalDefaultsSaferPathLeavesUnset(t *testing.T) {
	body := &spawnChildRequest{Provider: "claude"}
	applyChildApprovalDefaults(body, false)
	if body.PermissionMode != "" || body.RiskConfirmed {
		t.Fatalf("safer path should not fill bypass defaults: %+v", body)
	}

	explicit := &spawnChildRequest{Provider: "claude", PermissionMode: "bypassPermissions", RiskConfirmed: true}
	applyChildApprovalDefaults(explicit, false)
	if explicit.PermissionMode != "bypassPermissions" || !explicit.RiskConfirmed {
		t.Fatalf("safer path must preserve explicit values: %+v", explicit)
	}
}

// 承認ダイアログに出す実効権限は、applyChildApprovalDefaults を実際に通して作る。
// 対応表を書き写すと、既定を変えたときにダイアログだけ古い表示のまま残るため。
func TestChildApprovalPreviewMatchesAppliedDefaults(t *testing.T) {
	preview := childApprovalPreview(spawnChildRequest{Provider: "codex"}, true)
	for _, provider := range orchestrationProviders {
		want := &spawnChildRequest{Provider: provider}
		applyChildApprovalDefaults(want, true)
		got, ok := preview[provider]
		if !ok {
			t.Fatalf("provider %q が preview に無い", provider)
		}
		if got.PermissionMode != want.PermissionMode || got.Sandbox != want.Sandbox ||
			got.AskForApproval != want.AskForApproval || got.RiskConfirmed != want.RiskConfirmed {
			t.Fatalf("provider %q: preview = %+v, applied = %+v", provider, got, want)
		}
	}
	if codex := preview["codex"]; codex.Sandbox != "danger-full-access" || codex.AskForApproval != "never" || !codex.RiskConfirmed {
		t.Fatalf("codex はサンドボックス無効で起動するのに preview に出ていない: %+v", codex)
	}
}

// child_full_bypass が off のときは Hub が何も埋めない。ダイアログもそれを
// そのまま出せるよう、全 provider が空のまま返る。
func TestChildApprovalPreviewEmptyWhenBypassOff(t *testing.T) {
	preview := childApprovalPreview(spawnChildRequest{Provider: "codex"}, false)
	for provider, got := range preview {
		if got.PermissionMode != "" || got.Sandbox != "" || got.AskForApproval != "" || got.RiskConfirmed {
			t.Fatalf("provider %q: bypass off なのに埋まっている: %+v", provider, got)
		}
	}
}

// ダイアログは承認前に provider を差し替えられる。applySpawnConfirmationDecision は
// 承認者が差し替えた欄だけを置き換えて他は持ち越すので、呼び出し側が明示した値は
// 差し替え後の provider でも残る。preview もそのとおりに見せる。
func TestChildApprovalPreviewKeepsExplicitValuesAcrossProviders(t *testing.T) {
	preview := childApprovalPreview(spawnChildRequest{Provider: "codex", PermissionMode: "acceptEdits"}, true)
	if got := preview["claude"].PermissionMode; got != "acceptEdits" {
		t.Fatalf("明示値が provider 差し替え後も残るはず: %q", got)
	}
}

// 未知の provider もダイアログでは選択肢に残る。preview に入っていないと、
// その選択の間だけ権限欄が空白になる。
func TestChildApprovalPreviewCoversUnknownRequestedProvider(t *testing.T) {
	preview := childApprovalPreview(spawnChildRequest{Provider: "made-up-cli"}, true)
	if _, ok := preview["made-up-cli"]; !ok {
		t.Fatalf("未知の provider が preview に無い: %+v", preview)
	}
	if _, ok := preview["codex"]; !ok {
		t.Fatalf("既知の provider が落ちている: %+v", preview)
	}
}

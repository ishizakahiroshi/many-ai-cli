package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

// TestCustomProviderSessionFor は session.customProviderSession の計算式
// （wrapperLoop / reattach の両方が登録時に1度だけ呼ぶ）を、実プロセスや
// websocket 接続なしで直接固定する（plan_custom-provider-spawn-execution.md C4）。
func TestCustomProviderSessionFor(t *testing.T) {
	cfg := &config.Config{CustomProviders: config.CustomProviders{
		{ID: "my-cli", Command: "my-cli --agent"},
		{ID: "claude", Command: "claude"}, // built-in と衝突するので EffectiveCustomProviders から落ちる
	}}

	if !customProviderSessionFor(cfg, "my-cli") {
		t.Error("customProviderSessionFor(\"my-cli\") = false, want true (custom_providers entry)")
	}
	for _, provider := range []string{"claude", "codex", "shell", "unregistered-cli"} {
		if customProviderSessionFor(cfg, provider) {
			t.Errorf("customProviderSessionFor(%q) = true, want false", provider)
		}
	}
}

// TestSessionApprovalDetectionEligible は、session.customProviderSession が
// 零値（未設定）のままの built-in セッションに対して isAIProvider(Provider) 側の
// 判定が影響を受けないことを固定する。custom_provider_session フィールドを
// 追加した際、これを知らない既存の session{} リテラル（テストを含め65箇所超）が
// 零値 false のまま built-in の承認検出まで壊してしまう回帰が実際に発生した
// （TestReplayApprovalSkipsVTMarkerWhenTranscriptIsSource ほか）ため、
// この不変条件を明示的に固定する。
func TestSessionApprovalDetectionEligible(t *testing.T) {
	// customProviderSession を一切知らない、素の struct リテラル（既存の
	// テストヘルパーの大半がこの形）。
	claudeSes := &session{Provider: "claude"}
	if !sessionApprovalDetectionEligible(claudeSes) {
		t.Error("built-in provider session with zero-value customProviderSession must still be eligible")
	}

	shellSes := &session{Provider: "shell"}
	if sessionApprovalDetectionEligible(shellSes) {
		t.Error("shell session must not be eligible")
	}

	unknownSes := &session{Provider: "some-other-cli"}
	if sessionApprovalDetectionEligible(unknownSes) {
		t.Error("unrecognized provider without customProviderSession set must not be eligible")
	}

	customSes := &session{Provider: "my-cli", customProviderSession: true}
	if !sessionApprovalDetectionEligible(customSes) {
		t.Error("session with customProviderSession=true must be eligible")
	}

	if sessionApprovalDetectionEligible(nil) {
		t.Error("nil session must not be eligible, not panic")
	}
}

// TestActiveApprovalRuleSessionSnapsExcludesCustomProvider は、custom provider
// セッションが承認「検出」の対象になっても、フック注入スイープ（isAIProvider の
// まま据え置き）の対象には入らないことを固定する
// （plan_custom-provider-spawn-execution.md 決定事項5「フックは built-in のみ」の
// 機械的な歯止め）。
func TestActiveApprovalRuleSessionSnapsExcludesCustomProvider(t *testing.T) {
	s := newTestServer()
	s.sessions[1] = &session{ID: 1, Provider: "claude", CWD: "/proj/claude", State: "running"}
	s.sessions[2] = &session{ID: 2, Provider: "my-cli", CWD: "/proj/custom", State: "running", customProviderSession: true}
	s.wrappers[1] = &wrapperConn{}
	s.wrappers[2] = &wrapperConn{}

	snaps := s.activeApprovalRuleSessionSnaps()
	if len(snaps) != 1 {
		t.Fatalf("snaps = %#v, want exactly 1 (claude only)", snaps)
	}
	if snaps[0].provider != "claude" {
		t.Fatalf("snaps[0].provider = %q, want claude", snaps[0].provider)
	}
}

package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

// usage probe は Hub が内部で起こす使い捨て spawn なので、リスク確認の門で
// 止めてはならない。止めると RiskConfirmed が立たないまま必ず
// risk confirmation required で落ち、Usage パネルには数値が出ているのに
// 「取得できませんでした」だけが出る（2026-08-20 に発生）。
func TestUsageProbeSpawnSkipsRiskConfirmation(t *testing.T) {
	probe := spawnWrappedSpec{Provider: "claude", Model: config.DefaultUsageProbeModel, UsageProbe: true}
	if spawnNeedsRiskConfirmation(probe, true) {
		t.Fatal("usage probe spawn must not require risk confirmation")
	}
	user := spawnWrappedSpec{Provider: "claude", Model: "claude-opus-4-7"}
	if !spawnNeedsRiskConfirmation(user, true) {
		t.Fatal("user spawn that needs confirmation must be blocked")
	}
	confirmed := user
	confirmed.RiskConfirmed = true
	if spawnNeedsRiskConfirmation(confirmed, true) {
		t.Fatal("confirmed user spawn must proceed")
	}
	if spawnNeedsRiskConfirmation(user, false) {
		t.Fatal("user spawn without high risk must proceed")
	}
}

// probe のモデルは低コスト固定値でユーザーの選択ではない。既定モデルとして
// 保存すると、次にユーザーが自分のモデルで起動したときに「モデル変更＝高リスク」
// の確認が出る。
func TestUsageProbeSpawnDoesNotRecordLastModel(t *testing.T) {
	probe := spawnWrappedSpec{Provider: "claude", UsageProbe: true}
	if spawnRecordsLastModel(probe, config.DefaultUsageProbeModel, "") {
		t.Fatal("usage probe model must not become the user's next default")
	}
	user := spawnWrappedSpec{Provider: "claude"}
	if !spawnRecordsLastModel(user, "claude-opus-4-7", "") {
		t.Fatal("user spawn model must be recorded")
	}
	if spawnRecordsLastModel(user, "", "") {
		t.Fatal("empty model must not be recorded")
	}
	if spawnRecordsLastModel(user, "qwen3", RouteOllama) {
		t.Fatal("local route model must not be recorded")
	}
}

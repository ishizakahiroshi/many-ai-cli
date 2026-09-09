// handoff.go reports the state of ~/.many-ai-cli/handoff, the session
// handoff record store (internal/handoff), for `many-ai-cli doctor`.
//
// It always emits exactly one line — OK when disabled or empty, same as
// sessionLog() in doctor.go — so "nothing recorded yet" is visible and not
// confused with "this check never ran".
//
// Retention cleanup itself lives in the Hub's existing periodic maintenance
// loop (internal/hub/maintenance.go's cleanHandoff, mirroring cleanAttachments
// / cleanSessionLogs), which is what actually removes files older than
// handoff.retention_days. This check follows the same design rule as
// internal/doctor/residue.go: do not try to raise the hit rate of a
// kill-safe cleanup path — report state here so a stale file waiting for the
// next Hub start is visible, not silent.
package doctor

import (
	"fmt"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/handoff"
)

func handoffCheck(cfg *config.Config) Check {
	if !cfg.Handoff.EnabledOrDefault() {
		return Check{"handoff", OK, "引き継ぎ記録 (handoff) は無効です（handoff.enabled: false）", ""}
	}
	status, err := handoff.StatDir()
	if err != nil {
		return Check{"handoff", Warn, "handoff ディレクトリを確認できません: " + err.Error(),
			"~/.many-ai-cli/handoff の読み取り権限を確認してください"}
	}
	if !status.Exists {
		return Check{"handoff", OK, "引き継ぎ記録 (handoff) はまだありません", ""}
	}
	retentionDays := cfg.Handoff.RetentionDaysOrDefault()
	msg := fmt.Sprintf("引き継ぎ記録 (handoff) %d 件（最古 %s / 保持 %d 日）",
		status.Files, humanAge(status.OldestAge), retentionDays)
	if status.OldestAge > time.Duration(retentionDays)*24*time.Hour {
		return Check{"handoff", Warn, msg + "。保持期限を超えたファイルがあります",
			"Hub を起動すると次回の定期処理（内部の maintenance loop）で自動削除されます"}
	}
	return Check{"handoff", OK, msg, ""}
}

func humanAge(d time.Duration) string {
	days := int(d.Hours() / 24)
	if days <= 0 {
		return "1 日未満"
	}
	return fmt.Sprintf("%d 日前", days)
}

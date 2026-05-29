package hub

import "strings"

// riskSummary は spawn 時のリスク判定結果。codex / claude で同一構造のため共通化する。
type riskSummary struct {
	HighRisk bool
}

func evaluateCodexRisk(currentModel, nextModel, sandbox, approval string) riskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := sandbox == "danger-full-access" || approval == "never"
	return riskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

func evaluateClaudeRisk(currentModel, nextModel, permissionMode string) riskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := permissionMode == "bypassPermissions"
	return riskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

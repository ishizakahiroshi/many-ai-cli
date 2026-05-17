package hub

import "strings"

type codexRiskSummary struct {
	HighRisk bool
}

type claudeRiskSummary struct {
	HighRisk bool
}

func evaluateCodexRisk(currentModel, nextModel, sandbox, approval string) codexRiskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := sandbox == "danger-full-access" || approval == "never"
	return codexRiskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

func evaluateClaudeRisk(currentModel, nextModel, permissionMode string) claudeRiskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := permissionMode == "bypassPermissions"
	return claudeRiskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

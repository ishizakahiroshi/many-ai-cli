// custom_provider.go implements the custom_providers: diagnostic row.
//
// custom_providers: is a power-user config.yaml section
// (config.EffectiveCustomProviders). Unlike a built-in provider, whatever
// the user points Command at is not covered by many-ai-cli's own ToS /
// compliance decisions (D-01 / D-09 in reference_declined-directions.md) —
// this check exists purely to keep that visible. Same "only speak up when it
// applies" shape as commandCode / residue / subscriptions: an environment
// without custom_providers gets zero extra output.
package doctor

import (
	"fmt"
	"strings"

	"many-ai-cli/internal/config"
)

func customProviders(cfg *config.Config) []Check {
	effective := config.EffectiveCustomProviders(cfg.CustomProviders)
	if len(effective) == 0 {
		return nil
	}
	ids := make([]string, 0, len(effective))
	for _, p := range effective {
		ids = append(ids, p.ID)
	}
	checks := []Check{{
		"custom provider", OK,
		fmt.Sprintf(
			"玄人設定の custom_providers を %d 件使用中です（%s）。built-in provider と異なり、利用規約・実行結果の責任は利用者側です",
			len(ids), strings.Join(ids, ", ")),
		"",
	}}
	checks = append(checks, customProviderPath(effective))
	if check, ok := customProviderApprovalPatternNotice(effective); ok {
		checks = append(checks, check)
	}
	return checks
}

// customProviderPath confirms each custom provider's Command resolves to an
// executable on PATH — nothing more. It never runs the user's command (no
// --version probe, unlike providers()): doctor's own doc comment ("local,
// non-mutating diagnostics") stops at existence, and an arbitrary CLI's
// startup side effects are not doctor's to trigger.
func customProviderPath(effective config.CustomProviders) Check {
	var missing []string
	for _, p := range effective {
		argv, err := p.Argv()
		if err != nil || len(argv) == 0 {
			missing = append(missing, fmt.Sprintf("%s (command は分解できません)", p.ID))
			continue
		}
		if _, err := providerLookPath(argv[0]); err != nil {
			missing = append(missing, fmt.Sprintf("%s (%s)", p.ID, argv[0]))
		}
	}
	if len(missing) == 0 {
		return Check{"custom provider path", OK, "custom_providers の command は全て PATH 上に見つかりました", ""}
	}
	return Check{
		"custom provider path", Warn,
		"custom_providers の一部が PATH 上に見つかりません: " + strings.Join(missing, ", "),
		"該当コマンドをインストールするか PATH を確認し、many-ai-cli を再起動してください",
	}
}

// customProviderApprovalPatternNotice fires only when at least one entry sets
// approval_pattern_source, since that field is not wired to anything yet
// (plan_custom-provider-spawn-execution.md decision 4 — deferred to a later
// plan). internal/hub/approval_patterns.go's fetched per-provider pattern
// files DO reach a live detector — but only the browser-side one
// (web/src/app/approval.ts's providerApprovalTriggers / hasNativePromptHint),
// and only for the 7 built-in provider ids that array is keyed to; a custom
// provider's id is never a key there, so approval_pattern_source has nothing
// to plug into on that path either way. Custom sessions still get approval
// *detection* through a different route: internal/hub/approval_detector.go's
// Go-side heuristic (nativeApprovalTriggerTokens + nativeApprovalLooksValid)
// is generic text matching, not keyed to a provider allowlist, so it already
// runs for any approvalDetectionEligible session. This notice just keeps the
// approval_pattern_source gap visible to whoever wrote the field expecting
// it to already do something.
func customProviderApprovalPatternNotice(effective config.CustomProviders) (Check, bool) {
	var ids []string
	for _, p := range effective {
		if strings.TrimSpace(p.ApprovalPatternSource) != "" {
			ids = append(ids, p.ID)
		}
	}
	if len(ids) == 0 {
		return Check{}, false
	}
	return Check{
		"custom provider approval pattern", Warn,
		fmt.Sprintf(
			"approval_pattern_source を設定している custom_providers（%s）がありますが、現状この値は未使用です。承認検出はプロバイダ別の設定ファイルではなく、汎用の文言ヒューリスティックで行われます",
			strings.Join(ids, ", ")),
		"",
	}, true
}

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
	"os"
	"path/filepath"
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
// approval_pattern_source. internal/hub の syncCustomApprovalPatterns
// （Hub 起動時に非同期で実行）が source を fetch/読み込みして
// ~/.many-ai-cli/approval-patterns/<id>.json へ書き出し、ブラウザ側の
// providerApprovalTriggers がそれを消費する（plan_custom-provider-extension-triage.md
// C5）。doctor は「local, non-mutating diagnostics」の建前どおりここでは fetch を
// 一切行わず、このファイルが既に存在するかどうかだけを見る——存在すれば前回の
// Hub 起動で同期済み、無ければ「まだ Hub を経由して同期していない」か「source が
// 読めず失敗した」のどちらかで、後者は hub.log を見ないとここからは判別できない。
func customProviderApprovalPatternNotice(effective config.CustomProviders) (Check, bool) {
	var withSource []config.CustomProvider
	for _, p := range effective {
		if strings.TrimSpace(p.ApprovalPatternSource) != "" {
			withSource = append(withSource, p)
		}
	}
	if len(withSource) == 0 {
		return Check{}, false
	}
	var missing []string
	for _, p := range withSource {
		if !customApprovalPatternMirrorExists(p.ID) {
			missing = append(missing, p.ID)
		}
	}
	if len(missing) == 0 {
		return Check{
			"custom provider approval pattern", OK,
			fmt.Sprintf("approval_pattern_source を設定している custom_providers は全て同期済みです（%d 件）", len(withSource)),
			"",
		}, true
	}
	return Check{
		"custom provider approval pattern", Warn,
		fmt.Sprintf(
			"approval_pattern_source を設定しているのに未同期の custom_providers があります（%s）。Hub を起動（または再起動）すると同期を試みます。それでも消えない場合は source（絶対パスなら ~/.many-ai-cli 配下、URL なら https://raw.githubusercontent.com のみ許可）を確認し、hub.log を見てください",
			strings.Join(missing, ", ")),
		"",
	}, true
}

func customApprovalPatternMirrorExists(id string) bool {
	dir, err := config.Dir()
	if err != nil {
		return false
	}
	_, err = os.Stat(filepath.Join(dir, "approval-patterns", id+".json"))
	return err == nil
}

package config

import (
	"fmt"
	"strings"

	"many-ai-cli/internal/provider"
)

// LegacyProviderDefinitions converts the existing config.yaml custom provider
// entries into provider definitions without writing config.yaml or invoking a
// command. SplitCommandLine remains the only command parser, so old quoting and
// Windows path behavior stay unchanged during the registry migration.
func LegacyProviderDefinitions(raw CustomProviders) ([]provider.Definition, []provider.Diagnostic) {
	effective := EffectiveCustomProviders(raw)
	definitions := make([]provider.Definition, 0, len(effective))
	var diagnostics []provider.Diagnostic
	for _, legacy := range effective {
		argv, err := legacy.Argv()
		if err != nil || len(argv) == 0 {
			message := "legacy custom provider command cannot be split"
			if err != nil {
				message = fmt.Sprintf("legacy custom provider command cannot be split: %v", err)
			}
			diagnostics = append(diagnostics, provider.Diagnostic{
				Code:     "legacy_command_invalid",
				Severity: provider.SeverityWarning,
				Field:    legacy.ID + ".command",
				Message:  message,
			})
			continue
		}
		definition := provider.Definition{
			SchemaVersion:         provider.CurrentSchemaVersion,
			ID:                    legacy.ID,
			DisplayName:           legacy.EffectiveLabel(),
			ApprovalPatternSource: legacy.ApprovalPatternSource,
			Source: provider.SourceRef{
				Origin: provider.OriginLegacy,
			},
			Launch: &provider.LaunchDefinition{
				Executable: argv[0],
				Args:       append([]string(nil), argv[1:]...),
			},
		}
		if legacy.Headless != nil {
			if err := ValidateHeadlessDef(*legacy.Headless); err != nil {
				diagnostics = append(diagnostics, provider.Diagnostic{
					Code:     "legacy_headless_invalid",
					Severity: provider.SeverityWarning,
					Field:    legacy.ID + ".headless",
					Message:  "invalid headless definition was ignored",
				})
			} else {
				definition.Launch.Headless = &provider.HeadlessDefinition{
					Args:      append([]string(nil), legacy.Headless.Args...),
					Format:    strings.TrimSpace(legacy.Headless.Format),
					PromptVia: strings.TrimSpace(legacy.Headless.PromptVia),
				}
			}
		}
		definitions = append(definitions, definition)
	}
	return definitions, diagnostics
}

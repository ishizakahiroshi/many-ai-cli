package config

import (
	"strings"
	"testing"

	"many-ai-cli/internal/provider"
)

func TestLegacyProviderDefinitionsPreserveCommandArgvAndHeadless(t *testing.T) {
	definitions, diagnostics := LegacyProviderDefinitions(CustomProviders{{
		ID:                    "my-cli",
		Label:                 "My CLI",
		Command:               `"C:\\Program Files\\my-cli.exe" --agent`,
		ApprovalPatternSource: "https://example.test/patterns.json",
		Headless:              &HeadlessDef{Args: []string{"--print"}, Format: HeadlessFormatText, PromptVia: HeadlessPromptViaStdin},
	}})
	if len(diagnostics) != 0 {
		t.Fatalf("LegacyProviderDefinitions diagnostics = %#v", diagnostics)
	}
	if len(definitions) != 1 {
		t.Fatalf("LegacyProviderDefinitions returned %d definitions, want 1", len(definitions))
	}
	definition := definitions[0]
	if definition.Source.Origin != provider.OriginLegacy || definition.DisplayName != "My CLI" {
		t.Fatalf("legacy metadata = %#v", definition)
	}
	if definition.Launch == nil || definition.Launch.Executable != `C:\\Program Files\\my-cli.exe` || strings.Join(definition.Launch.Args, " ") != "--agent" {
		t.Fatalf("legacy launch = %#v", definition.Launch)
	}
	if definition.Launch.Headless == nil || definition.Launch.Headless.Format != HeadlessFormatText {
		t.Fatalf("legacy headless = %#v", definition.Launch.Headless)
	}
	if definition.ApprovalPatternSource == "" {
		t.Fatal("legacy approval pattern source was lost")
	}
}

func TestLegacyProviderDefinitionsDropsInvalidEntriesWithoutChangingInput(t *testing.T) {
	raw := CustomProviders{{ID: "bad", Command: `bad "unterminated`}, {ID: "claude", Command: "claude"}}
	definitions, diagnostics := LegacyProviderDefinitions(raw)
	if len(definitions) != 0 {
		t.Fatalf("LegacyProviderDefinitions = %#v, want no effective definitions", definitions)
	}
	if len(diagnostics) != 1 || diagnostics[0].Code != "legacy_command_invalid" {
		t.Fatalf("diagnostics = %#v, want command diagnostic for bad entry", diagnostics)
	}
	if raw[0].Command != `bad "unterminated` || raw[1].ID != "claude" {
		t.Fatalf("legacy input was modified: %#v", raw)
	}
}

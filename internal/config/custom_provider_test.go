package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

// TestCustomProvidersAbsentKeepsExistingBehaviour は `custom_providers:` を
// 書いていない既存 config.yaml が従来どおり読め、選択肢が増えないことを確認する。
func TestCustomProvidersAbsentKeepsExistingBehaviour(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	body := "token: abc123\nhub:\n  port: 47777\n"
	if err := os.WriteFile(filepath.Join(dir, "config.yaml"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	if cfg.CustomProviders != nil {
		t.Fatalf("CustomProviders = %#v, want nil for a config without the key", cfg.CustomProviders)
	}
	if len(EffectiveCustomProviders(cfg.CustomProviders)) != 0 {
		t.Fatalf("EffectiveCustomProviders = %#v, want none", EffectiveCustomProviders(cfg.CustomProviders))
	}
	if len(cfg.Warnings()) != 0 {
		t.Fatalf("Warnings() = %v, want none", cfg.Warnings())
	}
}

// TestCustomProvidersRoundTripThroughYAML はユーザーが書いた 1 件を正しくパースできることを確認する。
func TestCustomProvidersRoundTripThroughYAML(t *testing.T) {
	body := "custom_providers:\n" +
		"  - id: my-cli\n" +
		"    label: My CLI\n" +
		"    command: my-cli --agent\n" +
		"    approval_pattern_source: https://example.com/patterns.md\n"
	var cfg Config
	if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(cfg.CustomProviders) != 1 {
		t.Fatalf("CustomProviders = %#v, want 1 entry", cfg.CustomProviders)
	}
	got := cfg.CustomProviders[0]
	if got.ID != "my-cli" || got.Label != "My CLI" || got.Command != "my-cli --agent" || got.ApprovalPatternSource != "https://example.com/patterns.md" {
		t.Fatalf("CustomProviders[0] = %#v, unexpected", got)
	}
	effective := EffectiveCustomProviders(cfg.CustomProviders)
	if len(effective) != 1 || effective[0].ID != "my-cli" {
		t.Fatalf("EffectiveCustomProviders = %#v, want [my-cli]", effective)
	}
}

// TestCustomProvidersMalformedSectionDoesNotFailWholeConfig は壊れた
// `custom_providers:` が config 全体の破損（.bak 退避＋token 再生成）を
// 引き起こさないことを確認する。
func TestCustomProvidersMalformedSectionDoesNotFailWholeConfig(t *testing.T) {
	for _, body := range []string{
		"token: keepme\ncustom_providers: \"not a list\"\n",
		"token: keepme\ncustom_providers:\n  - 7\n",
		"token: keepme\ncustom_providers:\n  - label: no id or command\n",
		"token: keepme\ncustom_providers:\n  - id: my-cli\n",
		"token: keepme\ncustom_providers:\n  - command: my-cli\n",
	} {
		var cfg Config
		if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
			t.Fatalf("unmarshal(%q) returned an error; the whole config would have been discarded: %v", body, err)
		}
		if cfg.Token != "keepme" {
			t.Fatalf("token = %q for %q, want keepme", cfg.Token, body)
		}
		if len(cfg.CustomProviders) != 0 {
			t.Fatalf("CustomProviders = %#v for %q, want empty", cfg.CustomProviders, body)
		}
	}
}

func TestValidateCustomProviderID(t *testing.T) {
	valid := []string{"my-cli", "sub", "cli.v2", "a", "a.b_c-d", "0"}
	for _, id := range valid {
		if err := ValidateCustomProviderID(id); err != nil {
			t.Errorf("ValidateCustomProviderID(%q) = %v, want nil", id, err)
		}
	}
	invalid := []string{"", "-leading-dash", "Has Space", "UPPER", strings.Repeat("a", MaxCustomProviderIDLen+1)}
	for _, id := range invalid {
		if err := ValidateCustomProviderID(id); err == nil {
			t.Errorf("ValidateCustomProviderID(%q) = nil, want an error", id)
		}
	}
	for _, id := range BuiltinProviderIDs {
		if err := ValidateCustomProviderID(id); err == nil {
			t.Errorf("ValidateCustomProviderID(%q) = nil, want a built-in collision error", id)
		}
	}
}

func TestIsBuiltinProviderIDIsCaseInsensitive(t *testing.T) {
	if !IsBuiltinProviderID("Claude") {
		t.Fatal("IsBuiltinProviderID(\"Claude\") = false, want true")
	}
	if IsBuiltinProviderID("shell") {
		t.Fatal("IsBuiltinProviderID(\"shell\") = true, want false (not an AI CLI identity, see BuiltinProviderIDs doc)")
	}
	if IsBuiltinProviderID("my-cli") {
		t.Fatal("IsBuiltinProviderID(\"my-cli\") = true, want false")
	}
}

// TestCustomProviderWarningsSurfaceBrokenEntries は built-in 衝突・重複が
// Warnings() へ出て、かつ起動を止めない（Validate は通る）ことを確認する。
func TestCustomProviderWarningsSurfaceBrokenEntries(t *testing.T) {
	cfg := &Config{CustomProviders: CustomProviders{
		{ID: "my-cli", Command: "my-cli"},
		{ID: "MY-CLI", Command: "my-cli"}, // 正規化すると重複
		{ID: "claude", Command: "claude"}, // built-in と衝突
		{ID: "no-command"},                // command 欠落
	}}
	warnings := cfg.Warnings()
	if len(warnings) != 3 {
		t.Fatalf("Warnings() = %v, want 3 entries", warnings)
	}
	joined := strings.Join(warnings, "\n")
	for _, want := range []string{"duplicate", "collides", "missing command"} {
		if !strings.Contains(joined, want) {
			t.Errorf("warnings %q do not mention %q", joined, want)
		}
	}
	effective := EffectiveCustomProviders(cfg.CustomProviders)
	if len(effective) != 1 || effective[0].ID != "my-cli" {
		t.Fatalf("EffectiveCustomProviders = %#v, want only [my-cli]", effective)
	}
	// 警告があっても起動は止めない（Validate は LoadOrCreate と同じく applyDefaults の後に呼ばれる）。
	cfg.applyDefaults()
	if err := cfg.Validate(); err != nil {
		t.Fatalf("Validate() = %v; broken custom_providers entries must not stop the Hub", err)
	}
}

func TestCustomProviderEffectiveLabelFallsBackToID(t *testing.T) {
	if got := (CustomProvider{ID: "my-cli"}).EffectiveLabel(); got != "my-cli" {
		t.Fatalf("EffectiveLabel() = %q, want my-cli", got)
	}
	if got := (CustomProvider{ID: "my-cli", Label: "My CLI"}).EffectiveLabel(); got != "My CLI" {
		t.Fatalf("EffectiveLabel() = %q, want %q", got, "My CLI")
	}
}

func TestConfigCloneDeepCopiesCustomProviders(t *testing.T) {
	src := &Config{CustomProviders: CustomProviders{{ID: "my-cli", Command: "my-cli"}}}
	dst := src.Clone()
	dst.CustomProviders[0].Label = "changed"
	if src.CustomProviders[0].Label != "" {
		t.Fatal("Clone shared the CustomProviders slice")
	}
}

package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

// TestInfoCustomProvidersAbsentReturnsEmptyList は custom_providers 未設定時の
// /api/info が既存クライアントの前提（配列であること）を崩さないことを確認する。
func TestInfoCustomProvidersAbsentReturnsEmptyList(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"

	resp := callInfo(t, s)
	list, ok := resp["custom_providers"].([]any)
	if !ok {
		t.Fatalf("custom_providers = %#v (%T), want an empty array", resp["custom_providers"], resp["custom_providers"])
	}
	if len(list) != 0 {
		t.Fatalf("custom_providers = %v, want empty", list)
	}
}

// TestInfoCustomProvidersSurfacesEffectiveEntriesOnly は /api/info が
// EffectiveCustomProviders でフィルタ済みの一覧（id + ラベル）だけを返し、
// built-in と衝突するエントリは含めないことを確認する。
func TestInfoCustomProvidersSurfacesEffectiveEntriesOnly(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	s.cfg.CustomProviders = config.CustomProviders{
		{ID: "my-cli", Label: "My CLI", Command: "my-cli"},
		{ID: "claude", Command: "claude"}, // built-in と衝突するので除外される
	}

	resp := callInfo(t, s)
	list, ok := resp["custom_providers"].([]any)
	if !ok || len(list) != 1 {
		t.Fatalf("custom_providers = %#v, want exactly 1 entry", resp["custom_providers"])
	}
	entry, ok := list[0].(map[string]any)
	if !ok || entry["id"] != "my-cli" || entry["label"] != "My CLI" {
		t.Fatalf("custom_providers[0] = %#v, want {id: my-cli, label: My CLI}", list[0])
	}
}

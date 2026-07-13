package wrapper

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// 旧名 any-ai-cli マーカーで注入されたブロック（v0.3.x 以前のバイナリ由来）が、
// 新マーカーへの改名後も二重登録にならず移行・除去されることの回帰テスト。

func withTempCodexHome(t *testing.T) string {
	t.Helper()
	codexHome := t.TempDir()
	t.Setenv("CODEX_HOME", codexHome)
	return codexHome
}

func testUsageHookParams() UsageHookParams {
	return UsageHookParams{
		HubURL:    "http://127.0.0.1:47777",
		Token:     "deadbeef",
		SessionID: 7,
		ExePath:   "C:/dev/foo/many-ai-cli.exe",
	}
}

func legacyUsageHookBlock() string {
	return strings.Join([]string{
		legacyUsageHookBlockStart,
		"[[hooks.Stop]]",
		`command = "old-binary usage-relay --provider codex"`,
		legacyUsageHookBlockEnd,
		"",
	}, "\n")
}

func TestInjectCodexStopHookMigratesLegacyNamedBlock(t *testing.T) {
	codexHome := withTempCodexHome(t)
	path := filepath.Join(codexHome, "config.toml")
	content := "model = \"o3\"\n\n" + legacyUsageHookBlock()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	if err := InjectCodexStopHook(testUsageHookParams()); err != nil {
		t.Fatalf("InjectCodexStopHook failed: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	got := string(data)
	if strings.Contains(got, legacyUsageHookBlockStart) {
		t.Fatalf("legacy named block was not removed:\n%s", got)
	}
	if n := strings.Count(got, usageHookBlockStart); n != 1 {
		t.Fatalf("usage hook block count = %d, want 1\n%s", n, got)
	}
	if !strings.Contains(got, `model = "o3"`) {
		t.Fatalf("original content was not preserved:\n%s", got)
	}
}

func TestRemoveCodexStopHookRemovesLegacyNamedBlock(t *testing.T) {
	codexHome := withTempCodexHome(t)
	path := filepath.Join(codexHome, "config.toml")
	content := "model = \"o3\"\n\n" + legacyUsageHookBlock()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}

	if err := RemoveCodexStopHook(); err != nil {
		t.Fatalf("RemoveCodexStopHook failed: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	got := string(data)
	if strings.Contains(got, legacyUsageHookBlockStart) || strings.Contains(got, usageHookBlockStart) {
		t.Fatalf("usage hook block was not removed:\n%s", got)
	}
	if !strings.Contains(got, `model = "o3"`) {
		t.Fatalf("original content was not preserved:\n%s", got)
	}
}

func TestScanCodexStopHookInjectedDetectsLegacyNamedBlock(t *testing.T) {
	codexHome := withTempCodexHome(t)
	path := filepath.Join(codexHome, "config.toml")
	if err := os.WriteFile(path, []byte(legacyUsageHookBlock()), 0o600); err != nil {
		t.Fatal(err)
	}

	injected, err := ScanCodexStopHookInjected()
	if err != nil {
		t.Fatalf("ScanCodexStopHookInjected failed: %v", err)
	}
	if !injected {
		t.Fatal("legacy named block was not detected as injected")
	}
}

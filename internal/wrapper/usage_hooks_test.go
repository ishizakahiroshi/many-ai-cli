package wrapper

import (
	"encoding/json"
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
		ExePath:   "D:/dev/foo/many-ai-cli.exe",
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

// `--settings` は 1 回しか渡せないため、statusLine と crossSessionInbound は
// 同じ temp ファイルに同居する。要求されていないキーが混ざると、Claude の設定
// 階層でコマンドライン引数（local/project/user より上）として既定を上書きして
// しまうので、opts で指定したキーだけが書かれることを固定する。
func TestWriteClaudeSessionSettingsWritesOnlyRequestedKeys(t *testing.T) {
	cases := []struct {
		name     string
		opts     ClaudeSettingsOptions
		wantKeys []string
	}{
		{
			name:     "statusline only",
			opts:     ClaudeSettingsOptions{StatusLine: true},
			wantKeys: []string{"statusLine"},
		},
		{
			name:     "cross session inbound only",
			opts:     ClaudeSettingsOptions{CrossSessionInboundAccept: true},
			wantKeys: []string{"crossSessionInbound"},
		},
		{
			name:     "both",
			opts:     ClaudeSettingsOptions{StatusLine: true, CrossSessionInboundAccept: true},
			wantKeys: []string{"crossSessionInbound", "statusLine"},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if !tc.opts.enabled() {
				t.Fatal("opts.enabled() = false, want true")
			}
			path, cleanup, err := WriteClaudeSessionSettings(testUsageHookParams(), tc.opts)
			if err != nil {
				t.Fatalf("WriteClaudeSessionSettings failed: %v", err)
			}
			defer cleanup()

			data, err := os.ReadFile(path) // #nosec G304 -- テストが直前に書いた temp パス
			if err != nil {
				t.Fatal(err)
			}
			var doc map[string]any
			if err := json.Unmarshal(data, &doc); err != nil {
				t.Fatalf("settings is not valid JSON: %v\n%s", err, data)
			}
			if len(doc) != len(tc.wantKeys) {
				t.Fatalf("settings keys = %v, want %v", doc, tc.wantKeys)
			}
			for _, key := range tc.wantKeys {
				if _, ok := doc[key]; !ok {
					t.Fatalf("settings is missing key %q:\n%s", key, data)
				}
			}
			if tc.opts.CrossSessionInboundAccept && doc["crossSessionInbound"] != "accept" {
				t.Fatalf("crossSessionInbound = %v, want accept", doc["crossSessionInbound"])
			}

			cleanup()
			if _, err := os.Stat(path); !os.IsNotExist(err) {
				t.Fatalf("cleanup did not remove %s (err=%v)", path, err)
			}
		})
	}
}

func TestClaudeSettingsOptionsEnabledIsFalseWhenNothingRequested(t *testing.T) {
	if (ClaudeSettingsOptions{}).enabled() {
		t.Fatal("empty ClaudeSettingsOptions reported enabled")
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

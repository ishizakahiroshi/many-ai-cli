package config

import (
	"strings"
	"testing"
)

// 未設定時の既定は full＝今日の挙動。既定を bounded へ切り替えるのは、利用者が
// 段 2 で relay 1 周の完走を確認してからの別の判断
// （子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md C4）。
func TestChildPermissionDefaultTier(t *testing.T) {
	cases := map[string]string{
		"":          PermissionPresetFull,
		"  ":        PermissionPresetFull,
		"full":      PermissionPresetFull,
		"bounded":   PermissionPresetBounded,
		"attended":  PermissionPresetAttended,
		"whatever":  PermissionPresetFull,
		"Bounded":   PermissionPresetFull, // 大文字小文字は吸収しない（設定の誤りは誤りのまま warning にする）
		" bounded ": PermissionPresetBounded,
	}
	for raw, want := range cases {
		got := OrchestrationConfig{ChildPermissionDefault: raw}.ChildPermissionDefaultTier()
		if got != want {
			t.Fatalf("child_permission_default %q -> %q, want %q", raw, got, want)
		}
	}
}

func TestValidAllowedToolValue(t *testing.T) {
	ok := []string{"Read", "Edit", "Bash(git log *)", "shell(git status:*)", "Bash(bun run check)", "Bash(go test ./...)"}
	for _, v := range ok {
		if !ValidAllowedToolValue(v) {
			t.Fatalf("%q should be a valid allowlist entry", v)
		}
	}
	// コマンドラインへ出る値なので、シェル構文に読めるもの・フラグに化けるもの・
	// 区切りに使うコンマは通さない。
	bad := []string{"", "--allow-all", " Read", "Read;rm -rf /", "Read,Edit", "Read|Edit", "Bash(echo $HOME)", "Read\nEdit", strings.Repeat("A", MaxAllowedToolValueLen+1)}
	for _, v := range bad {
		if ValidAllowedToolValue(v) {
			t.Fatalf("%q should be rejected", v)
		}
	}
}

// 設定の誤りで Hub を止めない。効かなかったことだけを伝える（任意機能の作法）。
func TestChildPermissionWarnings(t *testing.T) {
	cfg := &Config{}
	cfg.Orchestration.ChildPermissionDefault = "boundeed"
	cfg.Orchestration.BoundedAllowedTools = map[string][]string{"claude": {"Read", "--oops"}}
	warnings := strings.Join(cfg.childPermissionWarnings(), "\n")
	if !strings.Contains(warnings, "child_permission_default") {
		t.Fatalf("warnings = %q, want the bad default named", warnings)
	}
	if !strings.Contains(warnings, "bounded_allowed_tools[claude]") || !strings.Contains(warnings, "--oops") {
		t.Fatalf("warnings = %q, want the dropped entry named", warnings)
	}

	clean := &Config{}
	clean.Orchestration.ChildPermissionDefault = PermissionPresetBounded
	if w := clean.childPermissionWarnings(); len(w) != 0 {
		t.Fatalf("valid settings must not warn: %v", w)
	}
}

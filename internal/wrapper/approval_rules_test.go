package wrapper

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func withTempHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return home
}

func TestInjectRulesSharedBlockIsIdempotentAcrossProviders(t *testing.T) {
	withTempHome(t)
	path := filepath.Join(t.TempDir(), "AGENTS.md")

	for _, provider := range []string{"codex", "copilot", "cursor-agent", "opencode"} {
		if err := InjectRules(provider, path); err != nil {
			t.Fatalf("InjectRules(%s) failed: %v", provider, err)
		}
	}
	if err := InjectRules("codex", path); err != nil {
		t.Fatalf("second InjectRules failed: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(data), sharedBlockStart); got != 1 {
		t.Fatalf("shared block count = %d, want 1\n%s", got, string(data))
	}
}

func TestRemoveRulesSharedBlockIsIdempotent(t *testing.T) {
	withTempHome(t)
	path := filepath.Join(t.TempDir(), "AGENTS.md")
	original := "before\n"
	if err := os.WriteFile(path, []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := InjectRules("copilot", path); err != nil {
		t.Fatalf("InjectRules failed: %v", err)
	}
	if err := RemoveRules("cursor-agent", path); err != nil {
		t.Fatalf("RemoveRules failed: %v", err)
	}
	if err := RemoveRules("codex", path); err != nil {
		t.Fatalf("second RemoveRules failed: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), sharedBlockStart) {
		t.Fatalf("shared block was not removed:\n%s", string(data))
	}
	if !strings.Contains(string(data), strings.TrimSpace(original)) {
		t.Fatalf("original content was not preserved:\n%s", string(data))
	}
}

func TestInjectRulesSharedBlockRefreshesStaleVersion(t *testing.T) {
	withTempHome(t)
	path := filepath.Join(t.TempDir(), "AGENTS.md")
	stale := strings.Join([]string{
		"before",
		"",
		sharedBlockStart,
		"<!-- version: 0 -->",
		"## many-ai-cli Approval Format",
		"(old rules)",
		sharedBlockEnd,
		"",
	}, "\n")
	if err := os.WriteFile(path, []byte(stale), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := InjectRules("codex", path); err != nil {
		t.Fatalf("InjectRules failed: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	content := string(data)
	if got := strings.Count(content, sharedBlockStart); got != 1 {
		t.Fatalf("shared block count = %d, want 1\n%s", got, content)
	}
	if strings.Contains(content, "<!-- version: 0 -->") {
		t.Fatalf("stale block was not replaced:\n%s", content)
	}
	if !strings.Contains(content, "<!-- version: "+rulesVersion+" -->") {
		t.Fatalf("current version block was not injected:\n%s", content)
	}
	if !strings.Contains(content, "before") {
		t.Fatalf("original content was not preserved:\n%s", content)
	}

	// 最新 version のブロックがある状態で再実行しても内容が変わらない（冪等）
	beforeSecond := content
	if err := InjectRules("codex", path); err != nil {
		t.Fatalf("second InjectRules failed: %v", err)
	}
	data2, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data2) != beforeSecond {
		t.Fatalf("InjectRules modified file with current block:\nbefore:\n%s\nafter:\n%s", beforeSecond, string(data2))
	}
}

func TestInjectRulesMigratesLegacyNamedBlock(t *testing.T) {
	withTempHome(t)
	path := filepath.Join(t.TempDir(), "AGENTS.md")
	legacy := strings.Join([]string{
		"before",
		"",
		legacySharedBlockStart,
		"<!-- version: " + rulesVersion + " -->",
		"## many-ai-cli Approval Format",
		"(rules injected by old any-ai-cli binary)",
		legacySharedBlockEnd,
		"",
	}, "\n")
	if err := os.WriteFile(path, []byte(legacy), 0o644); err != nil {
		t.Fatal(err)
	}

	if err := InjectRules("codex", path); err != nil {
		t.Fatalf("InjectRules failed: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	content := string(data)
	if strings.Contains(content, legacySharedBlockStart) {
		t.Fatalf("legacy named block was not removed:\n%s", content)
	}
	if got := strings.Count(content, sharedBlockStart); got != 1 {
		t.Fatalf("shared block count = %d, want 1\n%s", got, content)
	}
	if !strings.Contains(content, "before") {
		t.Fatalf("original content was not preserved:\n%s", content)
	}
}

// StripInjectedBlocks は doctor のハッシュ比較（sha256File）の前段で使われる。
// 3 種のブロックが揃って入っていても、地の文だけが残ることを確認する（C5）。
func TestStripInjectedBlocksRemovesAllThreeKinds(t *testing.T) {
	body := strings.Join([]string{
		"before",
		"",
		sharedBlockStart,
		"shared block content",
		sharedBlockEnd,
		"",
		legacySharedBlockStart,
		"legacy block content",
		legacySharedBlockEnd,
		"",
		delegationBlockStart,
		"delegation block content",
		delegationBlockEnd,
		"",
		"after",
	}, "\n")

	got := string(StripInjectedBlocks([]byte(body)))

	for _, marker := range []string{
		sharedBlockStart, sharedBlockEnd,
		legacySharedBlockStart, legacySharedBlockEnd,
		delegationBlockStart, delegationBlockEnd,
		"shared block content", "legacy block content", "delegation block content",
	} {
		if strings.Contains(got, marker) {
			t.Fatalf("StripInjectedBlocks left %q behind:\n%s", marker, got)
		}
	}
	if !strings.Contains(got, "before") || !strings.Contains(got, "after") {
		t.Fatalf("StripInjectedBlocks removed surrounding content:\n%s", got)
	}
}

// ブロックが無い入力は byte 単位でそのまま返る（除去し過ぎない）。
func TestStripInjectedBlocksReturnsInputUnchangedWhenNoBlocksPresent(t *testing.T) {
	body := []byte("plain content with no injected blocks\n")
	got := StripInjectedBlocks(body)
	if string(got) != string(body) {
		t.Fatalf("StripInjectedBlocks changed a block-free input:\nwant: %q\ngot:  %q", body, got)
	}
}

func TestInjectRulesClaudeImportIsIdempotent(t *testing.T) {
	withTempHome(t)
	path := filepath.Join(t.TempDir(), "CLAUDE.md")
	if err := InjectRules("claude", path); err != nil {
		t.Fatalf("InjectRules failed: %v", err)
	}
	if err := InjectRules("claude", path); err != nil {
		t.Fatalf("second InjectRules failed: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(data), claudeImportLine); got != 1 {
		t.Fatalf("claude import count = %d, want 1\n%s", got, string(data))
	}
}

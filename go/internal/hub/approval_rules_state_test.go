package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"golang.org/x/net/websocket"
)

func approvalStatePathForTest(t *testing.T) string {
	t.Helper()
	path, err := approvalRuleStatePath()
	if err != nil {
		t.Fatalf("approvalRuleStatePath: %v", err)
	}
	return path
}

// injectWithLiveSession は「セッションが 1 つ動いていて AGENTS.md にブロックが
// 注入されている」状態を作り、その Server と AGENTS.md のパスを返す。
func injectWithLiveSession(t *testing.T, project string) (*Server, string) {
	t.Helper()
	agentsPath := filepath.Join(project, "AGENTS.md")
	if err := os.WriteFile(agentsPath, []byte("# Project rules\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "copilot", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()
	s.injectApprovalRules()
	assertApprovalBlockCount(t, agentsPath, 1)
	return s, agentsPath
}

// TestRecoverOrphanedApprovalRules は Hub が kill されて注入ブロックが AGENTS.md に
// 残った状態からの回収を確かめる。修正前は in-memory の台帳ごと失われるため、
// ブロックは利用者のリポジトリに残り続けた（PlainSheet / trusted-context-mcp では
// それが commit されて公開リポジトリに載っていた）。
func TestRecoverOrphanedApprovalRules(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	_, agentsPath := injectWithLiveSession(t, project)

	statePath := approvalStatePathForTest(t)
	if _, err := os.Stat(statePath); err != nil {
		t.Fatalf("state file should exist while a target is injected: %v", err)
	}

	// Hub の kill を再現する: 後始末を呼ばないまま、新しい Server（空の in-memory 台帳）を作る。
	revived := newTestServer()
	revived.cfg.Approval.Enabled = true
	revived.recoverOrphanedApprovalRules()

	assertApprovalBlockCount(t, agentsPath, 0)
	if _, err := os.Stat(statePath); !os.IsNotExist(err) {
		t.Fatalf("state file should be gone after a full recovery, stat err = %v", err)
	}
	data, err := os.ReadFile(agentsPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "# Project rules\n" {
		t.Fatalf("AGENTS.md = %q, want the original content back", string(data))
	}
}

// TestRecoverOrphanedApprovalRulesRestoresProfileClaudeTarget は、宛先が既定の
// ~/.claude ではなく profile の CLAUDE.md だった場合も、その path のまま台帳へ
// 保存され、次回起動の回収が同じ path から import 行を外すことを確かめる。
// 台帳は path 単位なので実装は変えていないが、profile 宛先を入れた状態で
// 一度も通っていなかった経路なのでここで固定する。
func TestRecoverOrphanedApprovalRulesRestoresProfileClaudeTarget(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	claudeDir := t.TempDir()
	rulesPath := filepath.Join(claudeDir, "CLAUDE.md")
	original := "# Profile rules\n"
	if err := os.WriteFile(rulesPath, []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "claude", CWD: project, ClaudeDir: claudeDir, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()
	s.injectApprovalRules()
	assertClaudeImportCount(t, rulesPath, 1)

	statePath := approvalStatePathForTest(t)
	state, ok := s.readApprovalRuleState(statePath)
	if !ok || len(state.Targets) != 1 {
		t.Fatalf("state = %+v, ok = %v, want 1 target", state, ok)
	}
	if filepath.Clean(state.Targets[0].Path) != filepath.Clean(rulesPath) {
		t.Fatalf("persisted path = %q, want the profile CLAUDE.md", state.Targets[0].Path)
	}
	if state.Targets[0].Mode != approvalRuleModeClaudeImport {
		t.Fatalf("persisted mode = %q, want %q", state.Targets[0].Mode, approvalRuleModeClaudeImport)
	}

	// Hub の kill を再現する: 後始末を呼ばないまま新しい Server で回収する。
	revived := newTestServer()
	revived.cfg.Approval.Enabled = true
	revived.recoverOrphanedApprovalRules()

	assertClaudeImportCount(t, rulesPath, 0)
	data, err := os.ReadFile(rulesPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(data)) != strings.TrimSpace(original) {
		t.Fatalf("profile CLAUDE.md = %q, want the original content back", string(data))
	}
	if _, err := os.Stat(statePath); !os.IsNotExist(err) {
		t.Fatalf("state file should be gone after a full recovery, stat err = %v", err)
	}
}

// TestApprovalRuleStateClearedOnCleanShutdown は正常終了した場合に台帳が残らないこと
// （＝次回起動の回収が空振りしないこと）を確かめる。
func TestApprovalRuleStateClearedOnCleanShutdown(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	s, agentsPath := injectWithLiveSession(t, project)

	s.removeApprovalRules()

	assertApprovalBlockCount(t, agentsPath, 0)
	statePath := approvalStatePathForTest(t)
	if _, err := os.Stat(statePath); !os.IsNotExist(err) {
		t.Fatalf("state file should be gone after a clean shutdown, stat err = %v", err)
	}
}

// TestRecoverOrphanedApprovalRulesIgnoresCorruptState は壊れた台帳を根拠に
// 利用者の instruction file を書き換えないことを確かめる。
func TestRecoverOrphanedApprovalRulesIgnoresCorruptState(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	agentsPath := filepath.Join(project, "AGENTS.md")
	original := "# Project rules\n\n<!-- any-ai-cli:approval-rules -->\nleftover\n<!-- /any-ai-cli:approval-rules -->\n"
	if err := os.WriteFile(agentsPath, []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}
	statePath := approvalStatePathForTest(t)
	if err := os.MkdirAll(filepath.Dir(statePath), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(statePath, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.recoverOrphanedApprovalRules()

	data, err := os.ReadFile(agentsPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != original {
		t.Fatalf("AGENTS.md = %q, want it untouched when the state file is corrupt", string(data))
	}
}

// TestRecoverOrphanedApprovalRulesKeepsFailedEntries は外せなかった対象を台帳に
// 残し、次の起動でもう一度試せるようにしていることを確かめる。
func TestRecoverOrphanedApprovalRulesKeepsFailedEntries(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	_, agentsPath := injectWithLiveSession(t, project)

	statePath := approvalStatePathForTest(t)
	state, ok := newTestServer().readApprovalRuleState(statePath)
	if !ok || len(state.Targets) != 1 {
		t.Fatalf("state = %+v, ok = %v, want 1 target", state, ok)
	}
	// 削除に失敗する対象を 1 件足す。ディレクトリを指しておけば読み取りが
	// ErrNotExist ではないエラーで落ちる（存在しないパスは RemoveRules が
	// 正常終了扱いにするので、失敗の再現には使えない）。
	unreadable := filepath.Join(project, "UNREADABLE.md")
	if err := os.MkdirAll(unreadable, 0o700); err != nil {
		t.Fatal(err)
	}
	state.Targets = append(state.Targets, approvalRuleStateEntry{
		Path:      unreadable,
		Providers: []string{"copilot"},
		Mode:      approvalRuleModeSharedBlock,
	})
	if err := writeApprovalRuleState(statePath, state); err != nil {
		t.Fatal(err)
	}

	revived := newTestServer()
	revived.recoverOrphanedApprovalRules()

	// 実在した対象は外れている。
	assertApprovalBlockCount(t, agentsPath, 0)
	// 台帳は消えず、失敗分だけが残る。
	after, ok := revived.readApprovalRuleState(statePath)
	if !ok {
		t.Fatal("state file should still exist when an entry could not be recovered")
	}
	if len(after.Targets) != 1 || filepath.Base(after.Targets[0].Path) != "UNREADABLE.md" {
		t.Fatalf("remaining targets = %+v, want only the failed one", after.Targets)
	}
}

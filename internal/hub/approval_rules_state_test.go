package hub

import (
	"os"
	"path/filepath"
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

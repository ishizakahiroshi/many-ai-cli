package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"golang.org/x/net/websocket"
)

func withApprovalTestHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return home
}

func TestApprovalRulesSharedProjectTargetReferenceCount(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	agentsPath := filepath.Join(project, "AGENTS.md")
	if err := os.WriteFile(agentsPath, []byte("# Project rules\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, State: "running"}
	s.sessions[2] = &session{ID: 2, Provider: "cursor-agent", CWD: project, State: "running"}
	s.wrappers[1] = &websocket.Conn{}
	s.wrappers[2] = &websocket.Conn{}
	s.sessionsMu.Unlock()

	s.injectApprovalRules()
	assertApprovalBlockCount(t, agentsPath, 1)

	s.sessionsMu.Lock()
	delete(s.sessions, 1)
	delete(s.wrappers, 1)
	s.sessionsMu.Unlock()
	s.removeInactiveApprovalRules(providerApprovalRuleTargets("codex", project))
	assertApprovalBlockCount(t, agentsPath, 1)

	s.sessionsMu.Lock()
	delete(s.sessions, 2)
	delete(s.wrappers, 2)
	s.sessionsMu.Unlock()
	s.removeInactiveApprovalRules(providerApprovalRuleTargets("cursor-agent", project))
	assertApprovalBlockCount(t, agentsPath, 0)
}

func TestActiveApprovalRuleTargetsDedupesSharedAgentsPath(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	s := newTestServer()
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, State: "running"}
	s.sessions[2] = &session{ID: 2, Provider: "copilot", CWD: project, State: "running"}
	s.sessions[3] = &session{ID: 3, Provider: "cursor-agent", CWD: project, State: "running"}
	s.wrappers[1] = &websocket.Conn{}
	s.wrappers[2] = &websocket.Conn{}
	s.wrappers[3] = &websocket.Conn{}
	s.sessionsMu.Unlock()

	targets := s.activeApprovalRuleTargets()
	if len(targets) != 1 {
		t.Fatalf("target count = %d, want 1: %#v", len(targets), targets)
	}
	wantPath := filepath.Join(project, "AGENTS.md")
	if filepath.Clean(targets[0].Path) != filepath.Clean(wantPath) {
		t.Fatalf("target path = %q, want %q", targets[0].Path, wantPath)
	}
	if got := strings.Join(targets[0].Providers, ","); got != "codex,copilot,cursor-agent" {
		t.Fatalf("providers = %q, want codex,copilot,cursor-agent", got)
	}
}

func assertApprovalBlockCount(t *testing.T, path string, want int) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Count(string(data), "<!-- any-ai-cli:approval-rules -->"); got != want {
		t.Fatalf("approval block count = %d, want %d\n%s", got, want, string(data))
	}
}

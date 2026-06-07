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
	t.Setenv("CODEX_HOME", "")
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
	s.sessions[1] = &session{ID: 1, Provider: "copilot", CWD: project, State: "running"}
	s.sessions[2] = &session{ID: 2, Provider: "cursor-agent", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.wrappers[2] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	s.injectApprovalRules()
	assertApprovalBlockCount(t, agentsPath, 1)

	s.sessionsMu.Lock()
	delete(s.sessions, 1)
	delete(s.wrappers, 1)
	s.sessionsMu.Unlock()
	s.removeInactiveApprovalRules(providerApprovalRuleTargets("copilot", project))
	assertApprovalBlockCount(t, agentsPath, 1)

	s.sessionsMu.Lock()
	delete(s.sessions, 2)
	delete(s.wrappers, 2)
	s.sessionsMu.Unlock()
	s.removeInactiveApprovalRules(providerApprovalRuleTargets("cursor-agent", project))
	assertApprovalBlockCount(t, agentsPath, 0)
}

func TestCodexApprovalRulesUseGlobalAgents(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()
	s := newTestServer()
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	s.injectApprovalRules()

	globalAgentsPath := filepath.Join(home, ".codex", "AGENTS.md")
	assertApprovalBlockCount(t, globalAgentsPath, 1)
	if _, err := os.Stat(filepath.Join(project, "AGENTS.md")); !os.IsNotExist(err) {
		t.Fatalf("project AGENTS.md exists after codex injection: %v", err)
	}
}

func TestCodexInjectionRemovesLegacyProjectAgentsBlock(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()
	agentsPath := filepath.Join(project, "AGENTS.md")
	legacyBlock := strings.Join([]string{
		"# Project rules",
		"",
		"<!-- any-ai-cli:approval-rules -->",
		"legacy codex rules",
		"<!-- /any-ai-cli:approval-rules -->",
		"",
	}, "\n")
	if err := os.WriteFile(agentsPath, []byte(legacyBlock), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	s.injectApprovalRules()

	assertApprovalBlockCount(t, filepath.Join(home, ".codex", "AGENTS.md"), 1)
	assertApprovalBlockCount(t, agentsPath, 0)
}

func TestCodexApprovalRulesRespectCODEXHOME(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	codexHome := t.TempDir()
	t.Setenv("CODEX_HOME", codexHome)

	targets := providerApprovalRuleTargets("codex", project)
	if len(targets) != 1 {
		t.Fatalf("target count = %d, want 1: %#v", len(targets), targets)
	}
	wantPath := filepath.Join(codexHome, "AGENTS.md")
	if filepath.Clean(targets[0].Path) != filepath.Clean(wantPath) {
		t.Fatalf("target path = %q, want %q", targets[0].Path, wantPath)
	}
}

func TestActiveApprovalRuleTargetsSeparatesCodexGlobalAndProjectAgents(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()
	s := newTestServer()
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, State: "running"}
	s.sessions[2] = &session{ID: 2, Provider: "copilot", CWD: project, State: "running"}
	s.sessions[3] = &session{ID: 3, Provider: "cursor-agent", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.wrappers[2] = newWrapperConn(&websocket.Conn{})
	s.wrappers[3] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	targets := s.activeApprovalRuleTargets()
	if len(targets) != 2 {
		t.Fatalf("target count = %d, want 2: %#v", len(targets), targets)
	}
	wantProvidersByPath := map[string]string{
		filepath.Clean(filepath.Join(home, ".codex", "AGENTS.md")): "codex",
		filepath.Clean(filepath.Join(project, "AGENTS.md")):        "copilot,cursor-agent",
	}
	for _, target := range targets {
		want, ok := wantProvidersByPath[filepath.Clean(target.Path)]
		if !ok {
			t.Fatalf("unexpected target path %q in %#v", target.Path, targets)
		}
		if got := strings.Join(target.Providers, ","); got != want {
			t.Fatalf("providers for %q = %q, want %q", target.Path, got, want)
		}
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

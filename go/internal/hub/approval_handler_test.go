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
	// Hub 自身が profile 配下で起動している開発環境（CLAUDE_CONFIG_DIR が
	// 既に立っている）でも既定の宛先が ~/.claude/CLAUDE.md になるよう、
	// CODEX_HOME と同じく明示的に空にする。
	t.Setenv("CLAUDE_CONFIG_DIR", "")
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

func TestCodexApprovalRulesUseRegisteredSessionCodexHome(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()
	codexHome := t.TempDir()
	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "codex", CWD: project, CodexHome: codexHome, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	// The Hub process keeps its own environment, while the wrapper reports the
	// profile-specific CODEX_HOME in the register frame. Injection must follow
	// the reported session value instead of falling back to the Hub's default.
	s.injectApprovalRules()
	assertApprovalBlockCount(t, filepath.Join(codexHome, "AGENTS.md"), 1)
	if _, err := os.Stat(filepath.Join(home, ".codex", "AGENTS.md")); !os.IsNotExist(err) {
		t.Fatalf("Hub default AGENTS.md exists after profile injection: %v", err)
	}
}

func TestClaudeApprovalRulesRespectCLAUDECONFIGDIR(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	claudeDir := t.TempDir()
	t.Setenv("CLAUDE_CONFIG_DIR", claudeDir)

	targets := providerApprovalRuleTargets("claude", project)
	if len(targets) != 1 {
		t.Fatalf("target count = %d, want 1: %#v", len(targets), targets)
	}
	wantPath := filepath.Join(claudeDir, "CLAUDE.md")
	if filepath.Clean(targets[0].Path) != filepath.Clean(wantPath) {
		t.Fatalf("target path = %q, want %q", targets[0].Path, wantPath)
	}
}

func TestClaudeApprovalRulesDefaultToHomeClaude(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()

	// ClaudeDir も CLAUDE_CONFIG_DIR も無いときの宛先は従来どおり。
	targets := providerApprovalRuleTargets("claude", project)
	if len(targets) != 1 {
		t.Fatalf("target count = %d, want 1: %#v", len(targets), targets)
	}
	wantPath := filepath.Join(home, ".claude", "CLAUDE.md")
	if filepath.Clean(targets[0].Path) != filepath.Clean(wantPath) {
		t.Fatalf("target path = %q, want %q", targets[0].Path, wantPath)
	}
}

func TestClaudeApprovalRulesUseRegisteredSessionClaudeDir(t *testing.T) {
	home := withApprovalTestHome(t)
	project := t.TempDir()
	claudeDir := t.TempDir()
	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "claude", CWD: project, ClaudeDir: claudeDir, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	// Codex と同じ契約: Hub は自分の環境を持ったままなので、profile 固有の
	// CLAUDE_CONFIG_DIR は register frame で報告された値から取る。
	s.injectApprovalRules()
	assertClaudeImportCount(t, filepath.Join(claudeDir, "CLAUDE.md"), 1)
	if _, err := os.Stat(filepath.Join(home, ".claude", "CLAUDE.md")); !os.IsNotExist(err) {
		t.Fatalf("Hub default CLAUDE.md exists after profile injection: %v", err)
	}
}

// TestClaudeApprovalRulesRemovedFromProfileOnSessionEnd は profile の CLAUDE.md へ
// 付けた import 行が、そのセッションが消えたときに同じ宛先から外れることを固定する。
func TestClaudeApprovalRulesRemovedFromProfileOnSessionEnd(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	claudeDir := t.TempDir()
	rulesPath := filepath.Join(claudeDir, "CLAUDE.md")
	original := "# 利用者が書いた指示\n"
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

	s.sessionsMu.Lock()
	delete(s.sessions, 1)
	delete(s.wrappers, 1)
	s.sessionsMu.Unlock()
	s.removeInactiveApprovalRules(providerApprovalRuleTargetsWithHomes("claude", project, "", claudeDir))

	assertClaudeImportCount(t, rulesPath, 0)
	restored, err := os.ReadFile(rulesPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(restored)) != strings.TrimSpace(original) {
		t.Fatalf("撤去後に利用者の本文が変わっている:\n%q", restored)
	}
}

// TestClaudeApprovalRulesDoNotDuplicateThroughProfileLink は profile の CLAUDE.md が
// 既定側への symlink でも import 行が二重にならないことを確かめる。symlink 越しの
// 追記は実体（既定側）へ届き、次の注入は ScanClaudeConfigured が既存行を見つけて
// 何もしない。symlink を作れない環境ではコピー側のケースだけ走る。
func TestClaudeApprovalRulesDoNotDuplicateThroughProfileLink(t *testing.T) {
	inject := func(t *testing.T, project, claudeDir string) *Server {
		t.Helper()
		s := newTestServer()
		s.cfg.Approval.Enabled = true
		s.sessionsMu.Lock()
		s.sessions[1] = &session{ID: 1, Provider: "claude", CWD: project, ClaudeDir: claudeDir, State: "running"}
		s.wrappers[1] = newWrapperConn(&websocket.Conn{})
		s.sessionsMu.Unlock()
		s.injectApprovalRules()
		return s
	}

	t.Run("copy", func(t *testing.T) {
		home := withApprovalTestHome(t)
		project := t.TempDir()
		claudeDir := t.TempDir()
		defaultPath := filepath.Join(home, ".claude", "CLAUDE.md")
		if err := os.MkdirAll(filepath.Dir(defaultPath), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(defaultPath, []byte("# default\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(claudeDir, "CLAUDE.md"), []byte("# default\n"), 0o644); err != nil {
			t.Fatal(err)
		}

		s := inject(t, project, claudeDir)
		s.injectApprovalRules()

		assertClaudeImportCount(t, filepath.Join(claudeDir, "CLAUDE.md"), 1)
		// コピー運用では既定側には付かない（profile 側だけが宛先）。
		assertClaudeImportCount(t, defaultPath, 0)
	})

	t.Run("symlink", func(t *testing.T) {
		home := withApprovalTestHome(t)
		project := t.TempDir()
		claudeDir := t.TempDir()
		defaultPath := filepath.Join(home, ".claude", "CLAUDE.md")
		if err := os.MkdirAll(filepath.Dir(defaultPath), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(defaultPath, []byte("# default\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		linkPath := filepath.Join(claudeDir, "CLAUDE.md")
		if err := os.Symlink(defaultPath, linkPath); err != nil {
			// Windows で開発者モードが無い等、file symlink を作れない環境。
			// コピー側のケースは上の sub-test で必ず走っている。
			t.Skipf("file symlink を作れない環境のため skip: %v", err)
		}

		s := inject(t, project, claudeDir)
		s.injectApprovalRules()

		// 実体は 1 本しかないので、link 越しに読んでも実体を読んでも 1 行。
		assertClaudeImportCount(t, linkPath, 1)
		assertClaudeImportCount(t, defaultPath, 1)
	})
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
	// 新旧マーカー両方を数える（旧名 any-ai-cli ブロックの残存も検出するため）。
	got := strings.Count(string(data), "<!-- many-ai-cli:approval-rules -->") +
		strings.Count(string(data), "<!-- any-ai-cli:approval-rules -->")
	if got != want {
		t.Fatalf("approval block count = %d, want %d\n%s", got, want, string(data))
	}
}

// assertClaudeImportCount は claude の import 行方式（共有ブロックではなく 1 行追記）
// の注入数を数える。宛先が profile へ移っても二重に付かないことの確認に使う。
func assertClaudeImportCount(t *testing.T, path string, want int) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		if want == 0 && os.IsNotExist(err) {
			return
		}
		t.Fatal(err)
	}
	got := 0
	for _, line := range strings.Split(string(data), "\n") {
		if strings.TrimSpace(line) == "@~/.many-ai-cli/approval-rules.md" {
			got++
		}
	}
	if got != want {
		t.Fatalf("claude import line count = %d, want %d\n%s", got, want, string(data))
	}
}

// opencode は AGENTS.md を system prompt に載せることを 2026-08-29 に実測してから
// 注入対象へ入れた。承認ルールと委譲案内の両方がプロジェクト直下の AGENTS.md へ入り、
// セッションが消えたら両方とも外れて利用者の本文だけが残ることを固定する。
func TestOpenCodeApprovalAndDelegationUseProjectAgents(t *testing.T) {
	withApprovalTestHome(t)
	project := t.TempDir()
	agentsPath := filepath.Join(project, "AGENTS.md")
	original := "# 利用者が書いた指示\n"
	if err := os.WriteFile(agentsPath, []byte(original), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.cfg.Approval.Enabled = true
	s.cfg.UserPrefs.Spawn.DelegationAuto = true
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Provider: "opencode", CWD: project, State: "running"}
	s.wrappers[1] = newWrapperConn(&websocket.Conn{})
	s.sessionsMu.Unlock()

	s.injectApprovalRules()
	assertApprovalBlockCount(t, agentsPath, 1)

	injected, err := os.ReadFile(agentsPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(string(injected), original) {
		t.Fatalf("利用者が書いた本文を壊している:\n%s", injected)
	}
	if !strings.Contains(string(injected), "<!-- many-ai-cli:delegation -->") {
		t.Fatalf("委譲案内が入っていない:\n%s", injected)
	}

	s.sessionsMu.Lock()
	delete(s.sessions, 1)
	delete(s.wrappers, 1)
	s.sessionsMu.Unlock()
	s.removeApprovalTargets(providerApprovalRuleTargets("opencode", project))

	assertApprovalBlockCount(t, agentsPath, 0)
	restored, err := os.ReadFile(agentsPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(restored), "ai-cli:delegation") {
		t.Fatalf("委譲案内が置き去りになっている:\n%s", restored)
	}
	if strings.TrimSpace(string(restored)) != strings.TrimSpace(original) {
		t.Fatalf("撤去後に利用者の本文が変わっている:\n%q", restored)
	}
}

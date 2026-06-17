package hub

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func testConfigDir(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	dir, err := config.Dir()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	return dir
}

func writeTestConfigSourceFile(t *testing.T, name, text string) string {
	t.Helper()
	dir := testConfigDir(t)
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestParseSlashCmdsFromPlainDocs(t *testing.T) {
	text := `Command Purpose When to use it
/permissions Set what Codex can do without asking first.Relax or tighten approval requirements mid-session.
/model Choose the active model.Switch between models before running a task.
` + "`/status `Open the Settings interface (Status tab) showing version, model, account, and connectivity."

	cmds := parseSlashCmdsFromMarkdown(text)
	got := map[string]string{}
	for _, cmd := range cmds {
		got[cmd.Cmd] = cmd.Desc
	}

	for _, name := range []string{"/permissions", "/model", "/status"} {
		if got[name] == "" {
			t.Fatalf("expected %s to be parsed, got %#v", name, cmds)
		}
	}
}

func TestParseSlashCmdsStripsMarkdownInDesc(t *testing.T) {
	text := "" +
		"| `/agents` | Manage [agent](/en/sub-agents) configurations |\n" +
		"| `/batch` | **[Skill](/en/skills#bundled-skills).** Orchestrate things |\n" +
		"| `/btw` | Ask a quick [side question](/en/interactive-mode) about *something* |\n" +
		"| `/chrome` | Configure [Claude in Chrome](/en/chrome) settings |\n"
	cmds := parseSlashCmdsFromMarkdown(text)
	got := map[string]string{}
	for _, c := range cmds {
		got[c.Cmd] = c.Desc
	}

	checks := map[string]string{
		"/agents": "Manage agent configurations",
		"/btw":    "Ask a quick side question about something",
		"/chrome": "Configure Claude in Chrome settings",
	}
	for cmd, want := range checks {
		if got[cmd] != want {
			t.Fatalf("desc for %s: want %q, got %q", cmd, want, got[cmd])
		}
	}
	if !strings.Contains(got["/batch"], "Skill") || strings.Contains(got["/batch"], "[") || strings.Contains(got["/batch"], "*") {
		t.Fatalf("desc for /batch should be plain, got %q", got["/batch"])
	}
}

func TestCleanDescMarkdown(t *testing.T) {
	cases := map[string]string{
		"Manage [agent](/en/sub-agents) configurations":             "Manage agent configurations",
		"**[Skill](/en/skills#bundled-skills).** Orchestrate stuff": "Skill. Orchestrate stuff",
		"Use `tool` for things":                                     "Use tool for things",
		"*italic* and _under_ test":                                 "italic and under test",
		"managed-agents-onboard]":                                   "managed-agents-onboard]",
	}
	for in, want := range cases {
		if got := cleanDescMarkdown(in); got != want {
			t.Errorf("cleanDescMarkdown(%q): want %q, got %q", in, want, got)
		}
	}
}

func TestFetchAndParseSlashCmdsFromLocalFile(t *testing.T) {
	text := "| Command | Purpose | When to use it |\n" +
		"|---|---|---|\n" +
		"| `/permissions` | Set what Codex can do without asking first. | Relax or tighten approval requirements. |\n" +
		"| `/model` | Choose the active model. | Switch between models. |\n"
	path := writeTestConfigSourceFile(t, "codex.md", text)

	cmds, err := fetchAndParseSlashCmds(path)
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]string{}
	for _, cmd := range cmds {
		got[cmd.Cmd] = cmd.Desc
	}

	if got["/permissions"] != "Set what Codex can do without asking first" {
		t.Fatalf("unexpected /permissions desc: %q", got["/permissions"])
	}
	if got["/model"] != "Choose the active model" {
		t.Fatalf("unexpected /model desc: %q", got["/model"])
	}
}

func TestDiscoverSkillSlashCmdsFromCodexHome(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("CODEX_HOME", "")
	dir := filepath.Join(home, ".codex", "skills", "my-skill")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "SKILL.md")
	text := "---\nname: my-skill\ndescription: Use this for focused work.\n---\n# My Skill\n"
	if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
		t.Fatal(err)
	}

	cmds := discoverSkillSlashCmds("codex", skillSearchContext{HomeDir: home})
	if len(cmds) != 1 {
		t.Fatalf("expected 1 skill command, got %#v", cmds)
	}
	if cmds[0].Cmd != "$my-skill" || cmds[0].Kind != "skill" || cmds[0].Name != "my-skill" {
		t.Fatalf("unexpected skill command: %#v", cmds[0])
	}
	if !strings.Contains(cmds[0].Desc, "focused work") {
		t.Fatalf("unexpected desc: %q", cmds[0].Desc)
	}
}

func TestDiscoverSkillSlashCmdsFromClaudeHomeHonorsUserInvokable(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("CLAUDE_CONFIG_DIR", "")
	visibleDir := filepath.Join(home, ".claude", "skills", "release")
	hiddenDir := filepath.Join(home, ".claude", "skills", "internal-only")
	for _, dir := range []string{visibleDir, hiddenDir} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(visibleDir, "SKILL.md"), []byte("---\nname: release\ndescription: Ship a release.\n---\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(hiddenDir, "SKILL.md"), []byte("---\nname: internal-only\ndescription: Hidden.\nuser-invokable: false\n---\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cmds := discoverSkillSlashCmds("claude", skillSearchContext{HomeDir: home})
	if len(cmds) != 1 {
		t.Fatalf("expected 1 skill command, got %#v", cmds)
	}
	if cmds[0].Cmd != "/release" || cmds[0].Kind != "skill" || cmds[0].Name != "release" {
		t.Fatalf("unexpected skill command: %#v", cmds[0])
	}
}

func TestDiscoverSkillSlashCmdsUsesExplicitUserContext(t *testing.T) {
	hubHome := t.TempDir()
	userHome := t.TempDir()
	t.Setenv("HOME", hubHome)
	t.Setenv("USERPROFILE", hubHome)
	t.Setenv("CODEX_HOME", "")

	userSkillDir := filepath.Join(userHome, ".codex", "skills", "personal")
	if err := os.MkdirAll(userSkillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(userSkillDir, "SKILL.md"), []byte("---\nname: personal\ndescription: User-owned skill.\n---\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cmds := discoverSkillSlashCmds("codex", skillSearchContext{HomeDir: userHome})
	if len(cmds) != 1 || cmds[0].Cmd != "$personal" {
		t.Fatalf("expected user personal skill, got %#v", cmds)
	}

	hubCmds := discoverSkillSlashCmds("codex", skillSearchContext{})
	if len(hubCmds) != 0 {
		t.Fatalf("expected hub home fallback to be isolated, got %#v", hubCmds)
	}
}

func TestSkillSearchContextForRequestUsesSessionOwner(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	s.sessions[42] = &session{
		ID:        42,
		Provider:  "codex",
		HomeDir:   "/home/alice",
		CodexHome: "/home/alice/.codex-custom",
	}
	req := httptest.NewRequest(http.MethodGet, "/api/slash-commands?provider=codex&session_id=42&token=tok", nil)
	req.Host = "127.0.0.1:47777"

	ctx := s.skillSearchContextForRequest("codex", req)
	if ctx.HomeDir != "/home/alice" || ctx.CodexHome != "/home/alice/.codex-custom" {
		t.Fatalf("unexpected session owner context: %#v", ctx)
	}

	wrongProvider := s.skillSearchContextForRequest("claude", req)
	if wrongProvider != (skillSearchContext{}) {
		t.Fatalf("provider mismatch should not use session context: %#v", wrongProvider)
	}
}

func TestValidateSlashCmdSourceAllowsRawGitHubHTTPS(t *testing.T) {
	src := "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md"
	if err := validateSlashCmdSource(src); err != nil {
		t.Fatalf("expected raw GitHub source to be allowed: %v", err)
	}
}

func TestValidateSlashCmdSourceRejectsHTTPAndPrivateHosts(t *testing.T) {
	cases := []string{
		"http://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md",
		"https://127.0.0.1/slash.md",
		"https://169.254.169.254/latest/meta-data",
		"https://192.168.1.10/slash.md",
		"https://example.com/slash.md",
	}
	for _, src := range cases {
		if err := validateSlashCmdSource(src); err == nil {
			t.Fatalf("expected %q to be rejected", src)
		}
	}
}

func TestValidateSlashCmdSourceRejectsLocalFileOutsideConfigDir(t *testing.T) {
	path := filepath.Join(t.TempDir(), "codex.md")
	if err := os.WriteFile(path, []byte("# commands\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := validateSlashCmdSource(path); err == nil {
		t.Fatal("expected local source outside config dir to be rejected")
	}
}

func TestHandleSlashCmdSourcesRejectsInvalidSource(t *testing.T) {
	s := newSecTestServer(t, t.TempDir())
	body := []byte(`{"claude":"http://169.254.169.254/latest/meta-data","codex":""}`)
	req := httptest.NewRequest(http.MethodPost, "/api/slash-cmd-sources?token=tok", bytes.NewReader(body))
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()

	s.handleSlashCmdSources(w, req)

	if w.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d: %s", w.Code, w.Body.String())
	}
}

package hub

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"any-ai-cli/internal/config"
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

func TestValidateSlashCmdSourceAllowsRawGitHubHTTPS(t *testing.T) {
	src := "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md"
	if err := validateSlashCmdSource(src); err != nil {
		t.Fatalf("expected raw GitHub source to be allowed: %v", err)
	}
}

func TestValidateSlashCmdSourceRejectsHTTPAndPrivateHosts(t *testing.T) {
	cases := []string{
		"http://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md",
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

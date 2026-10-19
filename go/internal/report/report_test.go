package report

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestCollectUsesAllowlist(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.Port = 47777
	cfg.Token = "syntheticHubToken123"
	env := Collect(CollectOptions{
		Version:   "v0.synthetic",
		Provider:  "codex",
		Model:     "gpt-synthetic",
		UserAgent: "Synthetic Browser C:\\Users\\example-user\\profile",
		Config:    cfg,
	})
	if env.Provider != "codex" || env.Model != "gpt-synthetic" || env.AllowedConfig["hub_port"] != "47777" {
		t.Fatalf("Collect() = %#v", env)
	}
	joined := RenderEnvironment(env, "en")
	if strings.Contains(joined, cfg.Token) || strings.Contains(joined, "example-user") {
		t.Fatalf("RenderEnvironment leaked private data: %q", joined)
	}
}

func TestCollectRejectsUnknownProvider(t *testing.T) {
	if got := Collect(CollectOptions{Provider: "private-provider"}).Provider; got != "" {
		t.Fatalf("Provider = %q, want empty", got)
	}
}

func TestRenderMarkdownUsesEditableEnvironmentAndRedactsAgain(t *testing.T) {
	body := RenderMarkdown(TemplateInput{
		Locale:              "ja",
		Symptom:             "画面が止まる sk-syntheticSecret12345",
		Reproduction:        "再読込",
		EnvironmentMarkdown: "- OS: `windows`\n- token: syntheticToken123",
	})
	for _, want := range []string{"## 症状", "## 再現手順（任意）", "## 環境情報", "<REDACTED_SECRET>"} {
		if !strings.Contains(body, want) {
			t.Errorf("body missing %q: %q", want, body)
		}
	}
	for _, forbidden := range []string{"syntheticSecret12345", "syntheticToken123"} {
		if strings.Contains(body, forbidden) {
			t.Errorf("body retained %q: %q", forbidden, body)
		}
	}
}

func TestBuildIssueURLFixedDestinationAndLimit(t *testing.T) {
	got, tooLong := BuildIssueURL("synthetic", "body sk-syntheticSecret12345")
	if tooLong || !strings.HasPrefix(got, GitHubIssueBaseURL+"?") {
		t.Fatalf("BuildIssueURL() = %q, %v", got, tooLong)
	}
	if strings.Contains(got, "syntheticSecret12345") {
		t.Fatalf("URL retained secret: %q", got)
	}
	if got, tooLong := BuildIssueURL("synthetic", strings.Repeat("長", MaxIssueURLBytes)); got != "" || !tooLong {
		t.Fatalf("over limit BuildIssueURL() = %q, %v", got, tooLong)
	}
}

func TestSaveMarkdownUsesPrivateHomeRelativeFile(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	t.Setenv("USERPROFILE", tmp)
	display, err := SaveMarkdown("token: syntheticSaveToken123")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(display, filepath.Join("~", ".many-ai-cli", "reports")) {
		t.Fatalf("display path = %q", display)
	}
	name := filepath.Base(display)
	content, err := os.ReadFile(filepath.Join(tmp, ".many-ai-cli", "reports", name))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(content), "syntheticSaveToken123") {
		t.Fatalf("saved report retained secret: %q", content)
	}
}

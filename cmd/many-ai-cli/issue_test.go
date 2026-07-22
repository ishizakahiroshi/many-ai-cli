package main

import (
	"bytes"
	"errors"
	"io"
	"reflect"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestRunIssueDryRunPromptsForSymptomAndRedacts(t *testing.T) {
	deps, stdout, stderr := testIssueDependencies("approval modal fails\n")
	deps.collectMarkdown = func(_ *config.Config, provider, symptom string) string {
		if provider != "codex" {
			t.Fatalf("provider = %q, want codex", provider)
		}
		if symptom != "approval modal fails" {
			t.Fatalf("symptom = %q", symptom)
		}
		return "symptom: " + symptom + "\ntoken: sk-synthetic1234567890"
	}

	if err := runIssue(&config.Config{}, []string{"--provider", "codex", "--dry-run"}, deps); err != nil {
		t.Fatalf("runIssue() error = %v", err)
	}
	if strings.Contains(stdout.String(), "sk-synthetic") {
		t.Fatalf("dry-run output contains unredacted secret: %q", stdout.String())
	}
	if !strings.Contains(stdout.String(), "<REDACTED_SECRET>") {
		t.Fatalf("dry-run output missing redaction marker: %q", stdout.String())
	}
	if !strings.Contains(stderr.String(), "症状") {
		t.Fatalf("stderr = %q, want symptom prompt", stderr.String())
	}
}

func TestRunIssueAutoModeRejectsBeforeInteraction(t *testing.T) {
	deps, stdout, stderr := testIssueDependencies("")
	deps.getenv = func(key string) string {
		if key == "MANY_AI_CLI_AUTO" {
			return "1"
		}
		return ""
	}

	err := runIssue(&config.Config{}, []string{"--title", "problem"}, deps)
	if err == nil || !strings.Contains(err.Error(), "MANY_AI_CLI_AUTO=1") {
		t.Fatalf("runIssue() error = %v, want auto-mode rejection", err)
	}
	if stdout.Len() != 0 || stderr.Len() != 0 {
		t.Fatalf("unexpected interaction: stdout=%q stderr=%q", stdout.String(), stderr.String())
	}
}

func TestRunIssueTitleStillRequiresConfirmation(t *testing.T) {
	deps, stdout, _ := testIssueDependencies("n\n")
	runCalled := false
	deps.runCommand = func(string, []string, io.Writer, io.Writer) error {
		runCalled = true
		return nil
	}

	if err := runIssue(&config.Config{}, []string{"--title", "modal problem"}, deps); err != nil {
		t.Fatalf("runIssue() error = %v", err)
	}
	if runCalled {
		t.Fatal("external command ran after confirmation was declined")
	}
	if !strings.Contains(stdout.String(), "Title: modal problem") ||
		!strings.Contains(stdout.String(), "report for modal problem") ||
		!strings.Contains(stdout.String(), "cancelled") {
		t.Fatalf("stdout does not contain full preview and cancellation: %q", stdout.String())
	}
}

func TestRunIssueAcceptsPositionalTitle(t *testing.T) {
	deps, stdout, _ := testIssueDependencies("n\n")
	if err := runIssue(&config.Config{}, []string{"approval modal problem"}, deps); err != nil {
		t.Fatalf("runIssue() error = %v", err)
	}
	if !strings.Contains(stdout.String(), "Title: approval modal problem") {
		t.Fatalf("stdout = %q, want positional title in preview", stdout.String())
	}
}

func TestRunIssueRejectsConflictingTitleInputs(t *testing.T) {
	deps, _, _ := testIssueDependencies("")
	err := runIssue(&config.Config{}, []string{"--title", "flag title", "positional title"}, deps)
	if err == nil || !strings.Contains(err.Error(), "not both") {
		t.Fatalf("runIssue() error = %v, want title conflict", err)
	}
}

func TestRunIssueUsesGHWebAfterConfirmation(t *testing.T) {
	deps, stdout, _ := testIssueDependencies("y\n")
	deps.lookPath = func(name string) (string, error) {
		if name != "gh" {
			t.Fatalf("lookPath(%q), want gh", name)
		}
		return "/test/bin/gh", nil
	}
	var gotName string
	var gotArgs []string
	deps.runCommand = func(name string, args []string, _, _ io.Writer) error {
		gotName = name
		gotArgs = append([]string(nil), args...)
		return nil
	}

	if err := runIssue(&config.Config{}, []string{"--title", "modal problem", "--provider", "claude"}, deps); err != nil {
		t.Fatalf("runIssue() error = %v", err)
	}
	wantArgs := []string{
		"issue", "create", "--repo", issueRepository, "--web",
		"--title", "modal problem", "--body", "report for modal problem",
	}
	if gotName != "/test/bin/gh" || !reflect.DeepEqual(gotArgs, wantArgs) {
		t.Fatalf("command = %q %q, want %q %q", gotName, gotArgs, "/test/bin/gh", wantArgs)
	}
	if !strings.Contains(stdout.String(), "GitHub Issue preview") {
		t.Fatalf("stdout = %q, want preview", stdout.String())
	}
}

func TestRunIssuePrintsFixedURLWhenGHIsUnavailable(t *testing.T) {
	deps, stdout, _ := testIssueDependencies("y\n")
	deps.lookPath = func(string) (string, error) { return "", errors.New("not found") }
	deps.buildIssueURL = func(title, body string) (string, bool) {
		if title != "modal problem" || body != "report for modal problem" {
			t.Fatalf("BuildIssueURL(%q, %q) received unexpected content", title, body)
		}
		return "https://github.com/ishizakahiroshi/many-ai-cli/issues/new?title=safe&body=safe", false
	}

	if err := runIssue(&config.Config{}, []string{"--title", "modal problem"}, deps); err != nil {
		t.Fatalf("runIssue() error = %v", err)
	}
	if !strings.Contains(stdout.String(), "github.com/ishizakahiroshi/many-ai-cli/issues/new") {
		t.Fatalf("stdout = %q, want fixed issue URL", stdout.String())
	}
}

func TestRunIssueSavesLocalFallbackWhenURLIsTooLong(t *testing.T) {
	deps, _, _ := testIssueDependencies("y\n")
	deps.buildIssueURL = func(string, string) (string, bool) { return "", true }
	saved := ""
	deps.saveMarkdown = func(markdown string) (string, error) {
		saved = markdown
		return "~/.many-ai-cli/reports/report_test.md", nil
	}

	err := runIssue(&config.Config{}, []string{"--title", "modal problem"}, deps)
	if err == nil || !strings.Contains(err.Error(), "report_test.md") {
		t.Fatalf("runIssue() error = %v, want saved fallback path", err)
	}
	if saved != "report for modal problem" {
		t.Fatalf("saved markdown = %q", saved)
	}
}

func TestRunIssueSavesLocalFallbackWhenGHFails(t *testing.T) {
	deps, _, _ := testIssueDependencies("y\n")
	deps.runCommand = func(string, []string, io.Writer, io.Writer) error {
		return errors.New("synthetic gh failure")
	}
	saveCalled := false
	deps.saveMarkdown = func(string) (string, error) {
		saveCalled = true
		return "~/.many-ai-cli/reports/report_test.md", nil
	}

	err := runIssue(&config.Config{}, []string{"--title", "modal problem"}, deps)
	if err == nil || !strings.Contains(err.Error(), "report_test.md") || !saveCalled {
		t.Fatalf("runIssue() error = %v, saveCalled = %v", err, saveCalled)
	}
	if strings.Contains(err.Error(), "synthetic gh failure") {
		t.Fatalf("error leaks external details: %v", err)
	}
}

func TestRunIssueRejectsUnknownProvider(t *testing.T) {
	deps, _, _ := testIssueDependencies("")
	err := runIssue(&config.Config{}, []string{"--provider", "unknown", "--dry-run"}, deps)
	if err == nil || !strings.Contains(err.Error(), "unsupported provider") {
		t.Fatalf("runIssue() error = %v, want provider validation error", err)
	}
}

func testIssueDependencies(input string) (issueDependencies, *bytes.Buffer, *bytes.Buffer) {
	stdout := &bytes.Buffer{}
	stderr := &bytes.Buffer{}
	return issueDependencies{
		stdin:  strings.NewReader(input),
		stdout: stdout,
		stderr: stderr,
		getenv: func(string) string { return "" },
		lookPath: func(string) (string, error) {
			return "/test/bin/gh", nil
		},
		runCommand: func(string, []string, io.Writer, io.Writer) error {
			return nil
		},
		collectMarkdown: func(_ *config.Config, _, symptom string) string {
			return "report for " + symptom
		},
		defaultTitle:  func(symptom string) string { return symptom },
		buildIssueURL: func(string, string) (string, bool) { return "https://example.invalid", false },
		saveMarkdown:  func(string) (string, error) { return "report.md", nil },
	}, stdout, stderr
}

package main

import (
	"bufio"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/report"
)

const issueRepository = "ishizakahiroshi/many-ai-cli"

type issueDependencies struct {
	stdin           io.Reader
	stdout          io.Writer
	stderr          io.Writer
	getenv          func(string) string
	lookPath        func(string) (string, error)
	runCommand      func(string, []string, io.Writer, io.Writer) error
	collectMarkdown func(*config.Config, string, string) string
	defaultTitle    func(string) string
	buildIssueURL   func(string, string) (string, bool)
	saveMarkdown    func(string) (string, error)
}

func defaultIssueDependencies() issueDependencies {
	return issueDependencies{
		stdin:    os.Stdin,
		stdout:   os.Stdout,
		stderr:   os.Stderr,
		getenv:   os.Getenv,
		lookPath: exec.LookPath,
		runCommand: func(name string, args []string, stdout, stderr io.Writer) error {
			cmd := exec.Command(name, args...)
			cmd.Stdout = stdout
			cmd.Stderr = stderr
			return cmd.Run()
		},
		collectMarkdown: func(cfg *config.Config, provider, symptom string) string {
			env := report.Collect(report.CollectOptions{
				Version:  displayVersion(),
				Provider: provider,
				Config:   cfg,
			})
			return report.RenderMarkdown(report.TemplateInput{
				Locale:      "ja",
				Symptom:     symptom,
				Environment: env,
			})
		},
		defaultTitle:  report.DefaultTitle,
		buildIssueURL: report.BuildIssueURL,
		saveMarkdown:  report.SaveMarkdown,
	}
}

func runIssue(cfg *config.Config, args []string, deps issueDependencies) error {
	fs := flag.NewFlagSet("issue", flag.ContinueOnError)
	fs.SetOutput(deps.stderr)
	titleFlag := fs.String("title", "", "GitHub issue title")
	provider := fs.String("provider", "", "active provider")
	dryRun := fs.Bool("dry-run", false, "print the redacted report without opening GitHub")
	if err := fs.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return nil
		}
		return err
	}
	if fs.NArg() > 1 {
		return errors.New("issue accepts at most one positional title")
	}
	if fs.NArg() == 1 && strings.TrimSpace(*titleFlag) != "" {
		return errors.New("issue title must be provided either positionally or with --title, not both")
	}
	if !validIssueProvider(*provider) {
		return fmt.Errorf("unsupported provider %q", *provider)
	}
	if deps.getenv("MANY_AI_CLI_AUTO") == "1" && !*dryRun {
		return errors.New("issue is disabled when MANY_AI_CLI_AUTO=1; use --dry-run to inspect the report")
	}

	reader := bufio.NewReader(deps.stdin)
	requestedTitle := strings.TrimSpace(*titleFlag)
	if fs.NArg() == 1 {
		requestedTitle = strings.TrimSpace(fs.Arg(0))
	}
	symptom := requestedTitle
	if symptom == "" {
		if _, err := fmt.Fprint(deps.stderr, "症状を1行で入力してください: "); err != nil {
			return fmt.Errorf("write symptom prompt: %w", err)
		}
		line, err := reader.ReadString('\n')
		if err != nil && !errors.Is(err, io.EOF) {
			return fmt.Errorf("read symptom: %w", err)
		}
		symptom = strings.TrimSpace(line)
		if symptom == "" {
			return errors.New("symptom is required")
		}
	}

	markdown := report.Redact(deps.collectMarkdown(cfg, strings.TrimSpace(*provider), symptom))
	title := report.Redact(requestedTitle)
	if title == "" {
		title = report.Redact(deps.defaultTitle(symptom))
	}
	if *dryRun {
		_, err := fmt.Fprintln(deps.stdout, markdown)
		return err
	}

	if _, err := fmt.Fprintf(deps.stdout, "GitHub Issue preview\n\nTitle: %s\n\n%s\n", title, markdown); err != nil {
		return fmt.Errorf("write issue preview: %w", err)
	}
	confirmed, err := confirmIssue(reader, deps.stderr)
	if err != nil {
		return err
	}
	if !confirmed {
		_, err = fmt.Fprintln(deps.stdout, "Issue creation cancelled.")
		return err
	}

	// Re-run the scrubber immediately before any external handoff. This is a
	// deliberate final boundary even though report rendering is already safe.
	title = report.Redact(title)
	markdown = report.Redact(markdown)
	issueURL, tooLong := deps.buildIssueURL(title, markdown)
	if tooLong {
		return saveIssueFallback(deps, markdown, "report is too long for a GitHub issue URL")
	}

	ghPath, err := deps.lookPath("gh")
	if err != nil {
		_, err = fmt.Fprintln(deps.stdout, issueURL)
		return err
	}
	ghArgs := []string{
		"issue", "create",
		"--repo", issueRepository,
		"--web",
		"--title", title,
		"--body", markdown,
	}
	if err := deps.runCommand(ghPath, ghArgs, deps.stdout, deps.stderr); err != nil {
		return saveIssueFallback(deps, markdown, "gh could not open the issue preview")
	}
	return nil
}

func confirmIssue(reader *bufio.Reader, out io.Writer) (bool, error) {
	if _, err := fmt.Fprint(out, "この内容で GitHub の Issue 作成画面を開きますか? [y/N]: "); err != nil {
		return false, fmt.Errorf("write confirmation prompt: %w", err)
	}
	answer, err := reader.ReadString('\n')
	if err != nil && !errors.Is(err, io.EOF) {
		return false, fmt.Errorf("read confirmation: %w", err)
	}
	return strings.EqualFold(strings.TrimSpace(answer), "y"), nil
}

func saveIssueFallback(deps issueDependencies, markdown, reason string) error {
	displayPath, err := deps.saveMarkdown(report.Redact(markdown))
	if err != nil {
		return fmt.Errorf("%s; save local report: %w", reason, err)
	}
	return fmt.Errorf("%s; redacted report saved to %s", reason, displayPath)
}

func validIssueProvider(provider string) bool {
	switch strings.TrimSpace(provider) {
	case "", "claude", "codex", "copilot", "cursor-agent", "opencode", "grok":
		return true
	default:
		return false
	}
}

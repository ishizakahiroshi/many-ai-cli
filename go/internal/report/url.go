package report

import (
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const (
	GitHubIssueBaseURL = "https://github.com/ishizakahiroshi/many-ai-cli/issues/new"
	MaxIssueURLBytes   = 8192
)

// BuildIssueURL performs the mandatory final scrub and builds the fixed GitHub
// issue destination. The returned URL is empty when it exceeds the safe limit.
func BuildIssueURL(title, body string) (issueURL string, tooLong bool) {
	values := url.Values{}
	values.Set("title", Redact(strings.TrimSpace(title)))
	values.Set("body", Redact(body))
	issueURL = GitHubIssueBaseURL + "?" + values.Encode()
	if len([]byte(issueURL)) > MaxIssueURLBytes {
		return "", true
	}
	return issueURL, false
}

// SaveMarkdown stores a final-scrubbed fallback report under the user's home.
// It returns a home-relative display path so the local account name is never
// exposed to the browser or an error response.
func SaveMarkdown(body string) (displayPath string, err error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("resolve report directory: %w", err)
	}
	dir := filepath.Join(home, ".many-ai-cli", "reports")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", fmt.Errorf("create report directory: %w", err)
	}
	name := "report_" + time.Now().Format("20060102_150405.000000000") + ".md"
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(Redact(body)), 0o600); err != nil {
		return "", fmt.Errorf("save report: %w", err)
	}
	return filepath.Join("~", ".many-ai-cli", "reports", name), nil
}

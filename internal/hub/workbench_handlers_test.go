package hub

import (
	"strings"
	"testing"
)

func TestParseWorktreePorcelain(t *testing.T) {
	raw := strings.Join([]string{
		"worktree C:/dev/any-ai-cli",
		"HEAD abc123",
		"branch refs/heads/develop",
		"",
		"worktree C:/dev/any-ai-cli-feature",
		"HEAD def456",
		"detached",
		"",
	}, "\n")
	items := parseWorktreePorcelain(raw)
	if len(items) != 2 {
		t.Fatalf("len = %d, want 2: %#v", len(items), items)
	}
	if items[0]["branch"] != "develop" || items[1]["detached"] != "true" {
		t.Fatalf("items = %#v", items)
	}
}

func TestWorkbenchGitReviewHelpers(t *testing.T) {
	files := []map[string]string{
		{"status": "M", "path": "internal/hub/server.go"},
		{"status": "M", "path": "web/src/app/workbench.js"},
		{"status": "M", "path": "docs/local/plan.md"},
	}
	risks := gitReviewRisks(files, "", nil)
	if len(risks) == 0 || !strings.Contains(strings.Join(risks, "\n"), "Go source changed") {
		t.Fatalf("risks = %#v", risks)
	}
	split := strings.Join(gitCommitSplitSuggestions(files), "\n")
	for _, want := range []string{"backend", "frontend", "docs"} {
		if !strings.Contains(split, want) {
			t.Fatalf("split %q missing %q", split, want)
		}
	}
	tags := normalizeWorkbenchTags([]string{" review note ", "#review-note", "x y", "x y"})
	if len(tags) != 2 || tags[0] != "review-note" || tags[1] != "x-y" {
		t.Fatalf("tags = %#v", tags)
	}
}

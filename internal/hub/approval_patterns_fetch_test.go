package hub

import "testing"

func TestParseApprovalPatternsBasic(t *testing.T) {
	text := "# Claude Approval Patterns\n" +
		"\n" +
		"> 説明行はパース対象外。\n" +
		"\n" +
		"- `do you want to`\n" +
		"- `esc to cancel`\n" +
		"- `press enter to confirm or esc to go back`\n"
	got := parseApprovalPatternsFromMarkdown(text)
	want := []string{
		"do you want to",
		"esc to cancel",
		"press enter to confirm or esc to go back",
	}
	if len(got) != len(want) {
		t.Fatalf("len: want %d, got %d (%v)", len(want), len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("[%d]: want %q, got %q", i, want[i], got[i])
		}
	}
}

func TestParseApprovalPatternsDedup(t *testing.T) {
	text := "- `do you want to`\n" +
		"- `esc to cancel`\n" +
		"- `do you want to`\n"
	got := parseApprovalPatternsFromMarkdown(text)
	if len(got) != 2 {
		t.Fatalf("expected 2 entries after dedup, got %d (%v)", len(got), got)
	}
	if got[0] != "do you want to" || got[1] != "esc to cancel" {
		t.Fatalf("unexpected entries: %v", got)
	}
}

func TestParseApprovalPatternsIgnoresNonListLines(t *testing.T) {
	text := "# heading\n" +
		"\n" +
		"some prose with `backticks` should be ignored\n" +
		"> quote with `also ignored`\n" +
		"\n" +
		"- `valid pattern`\n"
	got := parseApprovalPatternsFromMarkdown(text)
	if len(got) != 1 || got[0] != "valid pattern" {
		t.Fatalf("unexpected entries: %v", got)
	}
}

func TestParseApprovalPatternsTrimsWhitespace(t *testing.T) {
	text := "  -   `padded pattern`   \n" +
		"-\t`tab indented`\t\n"
	got := parseApprovalPatternsFromMarkdown(text)
	want := []string{"padded pattern", "tab indented"}
	if len(got) != len(want) {
		t.Fatalf("len: want %d, got %d (%v)", len(want), len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("[%d]: want %q, got %q", i, want[i], got[i])
		}
	}
}

func TestParseApprovalPatternsUTF8(t *testing.T) {
	text := "- `この操作を許可`\n" +
		"- `続行しますか`\n" +
		"- `↑/↓ to change`\n"
	got := parseApprovalPatternsFromMarkdown(text)
	want := []string{"この操作を許可", "続行しますか", "↑/↓ to change"}
	if len(got) != len(want) {
		t.Fatalf("len: want %d, got %d (%v)", len(want), len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("[%d]: want %q, got %q", i, want[i], got[i])
		}
	}
}

func TestFetchAndParseApprovalPatternsFromLocalFile(t *testing.T) {
	text := "# Claude Approval Patterns\n\n" +
		"- `do you want to`\n" +
		"- `esc to cancel`\n"
	path := writeTestConfigSourceFile(t, "claude.md", text)
	got, err := fetchAndParseApprovalPatterns(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0] != "do you want to" || got[1] != "esc to cancel" {
		t.Fatalf("unexpected: %v", got)
	}
}

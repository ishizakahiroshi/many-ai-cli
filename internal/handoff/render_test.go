package handoff

import (
	"strconv"
	"strings"
	"testing"
)

// TestRenderMarkdownIncludesAllSections is the C1 completion criterion: for a
// session with completions, changes, and an intent, all six sections listed
// in the child plan (docs/local/plan_session-handoff-board_c5_handoff-md.md
// 内部 C1) appear.
func TestRenderMarkdownIncludesAllSections(t *testing.T) {
	records := []Record{
		{Kind: KindSessionStart, SessionID: 42, Provider: "claude", CWD: `C:\work\sample-repo`, Branch: "main", Model: "opus", SubscriptionID: "profile-1", TS: "2026-09-08T10:00:00+09:00"},
		{Kind: KindDone, Text: "[success] fixed the thing", TS: "2026-09-08T10:05:00+09:00"},
		{Kind: KindGitTurn, Turn: 1, Commit: "abcdef0123456789", CommitSubject: "fix bug", Files: []string{"a.go", "b.go"}},
		{Kind: KindIntent, Text: "次: 次のPRをレビューする / 未検証: DBスキーマの影響範囲"},
		{Kind: KindSessionEnd, Text: "completed", TS: "2026-09-08T11:00:00+09:00"},
	}

	got := RenderMarkdown(42, records)

	for _, want := range []string{
		"セッションの素性",
		"作業中の md",
		"直近の完了",
		"直近の変更",
		"次の一手",
		"後継への指示",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("markdown missing section %q:\n%s", want, got)
		}
	}
	if !strings.Contains(got, "claude") || !strings.Contains(got, `C:\work\sample-repo`) || !strings.Contains(got, "main") || !strings.Contains(got, "opus") {
		t.Errorf("markdown missing session identity fields:\n%s", got)
	}
	if !strings.Contains(got, "[success] fixed the thing") {
		t.Errorf("markdown does not carry the done Text verbatim:\n%s", got)
	}
	if !strings.Contains(got, "fix bug") || !strings.Contains(got, "a.go") || !strings.Contains(got, "b.go") {
		t.Errorf("markdown missing git turn fields:\n%s", got)
	}
	if !strings.Contains(got, "次のPRをレビューする") {
		t.Errorf("markdown missing intent text:\n%s", got)
	}
}

// TestRenderMarkdownDoesNotParseDoneClassificationPrefix is the C1 completion
// criterion covering the explicit "must not strip" rule: the "[kind] " prefix
// recordHandoffDone adds must survive unparsed in the rendered line.
func TestRenderMarkdownDoesNotParseDoneClassificationPrefix(t *testing.T) {
	records := []Record{
		{Kind: KindDone, Text: "[failure] something broke", TS: "2026-09-08T10:05:00+09:00"},
	}
	got := RenderMarkdown(1, records)
	if !strings.Contains(got, "[failure] something broke") {
		t.Fatalf("expected the done line to carry the [kind] prefix unparsed, got:\n%s", got)
	}
}

// TestRenderMarkdownNoWorkDocSaysNoRecord is the C1 completion criterion for
// a session with no WorkDoc field on any record: the section must say so
// honestly rather than being silently omitted.
func TestRenderMarkdownNoWorkDocSaysNoRecord(t *testing.T) {
	records := []Record{
		{Kind: KindSessionStart, Provider: "codex"},
	}
	got := RenderMarkdown(1, records)
	if !strings.Contains(got, "記録なし") {
		t.Fatalf("expected 記録なし for a missing WorkDoc, got:\n%s", got)
	}
}

// TestRenderMarkdownNoIntentIsHonestAboutWhy is the C1 completion criterion:
// a session with zero kind=intent records still produces a md, and the
// section explains the gap rather than inventing a next step.
func TestRenderMarkdownNoIntentIsHonestAboutWhy(t *testing.T) {
	records := []Record{
		{Kind: KindSessionStart, Provider: "claude"},
		{Kind: KindDone, Text: "did something", TS: "2026-09-08T10:00:00+09:00"},
	}
	got := RenderMarkdown(1, records)
	if !strings.Contains(got, "記録なし") {
		t.Fatalf("expected 記録なし in the next-step section, got:\n%s", got)
	}
	if !strings.Contains(got, "なぜ") {
		t.Fatalf("expected the honest caveat about not knowing why, got:\n%s", got)
	}
}

// TestRenderMarkdownDropsOldestDoneEntriesWithPointer is the C1 completion
// criterion: beyond RenderMaxDoneEntries, older completions are replaced by a
// "他 N 件" pointer rather than being truncated mid-content, and the newest
// entries are the ones kept.
func TestRenderMarkdownDropsOldestDoneEntriesWithPointer(t *testing.T) {
	var records []Record
	total := RenderMaxDoneEntries + 3
	for i := 0; i < total; i++ {
		records = append(records, Record{Kind: KindDone, Text: "done-" + strconv.Itoa(i), TS: "2026-09-08T10:00:00+09:00"})
	}
	got := RenderMarkdown(1, records)
	if !strings.Contains(got, "他 3 件") {
		t.Fatalf("expected a '他 3 件' pointer for the dropped entries, got:\n%s", got)
	}
	// The newest entry (last appended) must be present…
	if !strings.Contains(got, "done-"+strconv.Itoa(total-1)) {
		t.Fatalf("expected the newest done entry to survive truncation, got:\n%s", got)
	}
	// …and the oldest must not be (it was dropped, not just reordered).
	if strings.Contains(got, "done-0\n") {
		t.Fatalf("expected the oldest done entry to be dropped, got:\n%s", got)
	}
}

// TestRenderMarkdownNeverCarriesFieldsRecordCannotHold documents that this
// package cannot leak a diff/PTY/env body: Record itself has no such field
// (親 plan 不変条件 1, enforced by handoff_test.go's
// TestRecordFieldsAreTheAllowlist), so nothing supplied to RenderMarkdown can
// carry one through. This test guards against a future regression where a
// caller might be tempted to pass extra context alongside records.
func TestRenderMarkdownNeverCarriesFieldsRecordCannotHold(t *testing.T) {
	secretLike := "+ SECRET_API_KEY=sk-ant-should-never-appear"
	records := []Record{
		{Kind: KindGitTurn, Commit: "abc123", CommitSubject: "ok", Files: []string{"a.go"}},
		{Kind: KindDone, Text: "unrelated", TS: "2026-09-08T10:00:00+09:00"},
	}
	got := RenderMarkdown(1, records)
	if strings.Contains(got, secretLike) {
		t.Fatalf("markdown must never contain diff-body-shaped text it was never given:\n%s", got)
	}
}

// TestRenderMarkdownHandoffFromNotesPredecessor is the internal-C3
// completion criterion surfaced here: when the session's own session_start
// carries HandoffFrom, the rendered identity section and instructions
// mention the predecessor.
func TestRenderMarkdownHandoffFromNotesPredecessor(t *testing.T) {
	records := []Record{
		{Kind: KindSessionStart, Provider: "codex", HandoffFrom: 7},
	}
	got := RenderMarkdown(9, records)
	if !strings.Contains(got, "#7") {
		t.Fatalf("expected the predecessor session id to appear, got:\n%s", got)
	}
}

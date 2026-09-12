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

// TestRenderMarkdownTranscriptSectionOnlyWhenRecorded is the 内部 C1 completion
// criterion of 子 plan docs/local/plan_derived-session-launch_c4_handoff-routes.md:
// the predecessor's conversation log appears as a path plus how to read it, and
// a provider whose transcript location cannot be resolved (grok / copilot / …)
// gets no section at all rather than an empty one.
func TestRenderMarkdownTranscriptSectionOnlyWhenRecorded(t *testing.T) {
	const path = `C:\fake-home\.claude\projects\C--work-sample-repo\11111111-2222-4333-8444-555555555555.jsonl`
	withTranscript := RenderMarkdown(1, []Record{
		{Kind: KindSessionStart, Provider: "claude", Transcript: path},
	})
	if !strings.Contains(withTranscript, "前任の会話ログ") || !strings.Contains(withTranscript, path) {
		t.Fatalf("expected the transcript section and its path, got:\n%s", withTranscript)
	}
	if !strings.Contains(withTranscript, "末尾") {
		t.Fatalf("expected the section to say how to read the file, got:\n%s", withTranscript)
	}

	without := RenderMarkdown(1, []Record{{Kind: KindSessionStart, Provider: "grok"}})
	if strings.Contains(without, "前任の会話ログ") {
		t.Fatalf("a provider with no resolvable transcript must get no section, got:\n%s", without)
	}
}

// TestRenderMarkdownTranscriptTakesTheNewestRecord fixes which record wins when
// more than one carries a path: a session_end recorded after the session's log
// moved must not be overridden by the stale session_start value.
func TestRenderMarkdownTranscriptTakesTheNewestRecord(t *testing.T) {
	got := RenderMarkdown(1, []Record{
		{Kind: KindSessionStart, Provider: "codex", Transcript: "/fake/sessions/2026/09/12/rollout-old.jsonl"},
		{Kind: KindSessionEnd, Transcript: "/fake/sessions/2026/09/12/rollout-new.jsonl"},
	})
	if !strings.Contains(got, "rollout-new.jsonl") {
		t.Fatalf("expected the newest transcript path, got:\n%s", got)
	}
	if strings.Contains(got, "rollout-old.jsonl") {
		t.Fatalf("expected the older transcript path to be replaced, got:\n%s", got)
	}
}

// TestRenderMarkdownNoteSectionOnlyWhenRecorded is the 内部 C2 completion
// criterion: the memo the predecessor wrote is pointed at (and marked as the
// thing to read first), and a session that was never asked for one gets no
// section.
func TestRenderMarkdownNoteSectionOnlyWhenRecorded(t *testing.T) {
	const path = `C:\fake-home\.many-ai-cli\handoff\s42.note.md`
	withNote := RenderMarkdown(42, []Record{
		{Kind: KindSessionStart, Provider: "claude"},
		{Kind: KindNote, Note: path},
	})
	if !strings.Contains(withNote, "引き継ぎメモ") || !strings.Contains(withNote, path) {
		t.Fatalf("expected the note section and its path, got:\n%s", withNote)
	}
	if !strings.Contains(withNote, "先に") {
		t.Fatalf("expected the note section to say it is read first, got:\n%s", withNote)
	}

	without := RenderMarkdown(42, []Record{{Kind: KindSessionStart, Provider: "claude"}})
	if strings.Contains(without, "引き継ぎメモ") {
		t.Fatalf("a session with no memo must get no section, got:\n%s", without)
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

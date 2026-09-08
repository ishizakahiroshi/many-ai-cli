package hub

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/handoff"
	"many-ai-cli/internal/proto"
)

// withTempHandoffHome points internal/handoff (via config.Dir -> os.UserHomeDir)
// at a throwaway directory, mirroring internal/handoff/handoff_test.go's own
// helper. Each hub-layer test needs its own isolated ~/.many-ai-cli/handoff.
func withTempHandoffHome(t *testing.T) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
}

func readHandoffRecords(t *testing.T, sessionID int) []handoff.Record {
	t.Helper()
	path, err := handoff.PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	records, err := handoff.ReadAll(path)
	if err != nil {
		t.Fatalf("ReadAll: %v", err)
	}
	return records
}

// TestHandoffSessionStartEndIgnoresSessionLogEnabled is the C2 completion
// criterion for the internal C1 (「セッションの開始と終了」): a session's
// handoff lines must appear even when log.session_enabled is false, which is
// the shipped default. Widening this gate to depend on SessionEnabled would
// leave every default-configuration user with an empty board.
func TestHandoffSessionStartEndIgnoresSessionLogEnabled(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	s.cfg.Log.SessionEnabled = false // the shipped default; set explicitly to document the guarantee

	const sessionID = 101
	s.recordHandoffSessionStart(sessionID, "claude", `C:\work\sample-repo`, "main", "opus", "profile-1", 0)
	s.recordHandoffSessionEnd(sessionID, "completed", "")

	if s.cfg.Log.SessionEnabled {
		t.Fatal("test corrupted its own precondition")
	}

	got := readHandoffRecords(t, sessionID)
	if len(got) != 2 {
		t.Fatalf("got %d records, want 2 (session_start + session_end): %+v", len(got), got)
	}
	start, end := got[0], got[1]
	if start.Kind != handoff.KindSessionStart {
		t.Errorf("record 0 kind = %q, want %q", start.Kind, handoff.KindSessionStart)
	}
	if start.Provider != "claude" || start.CWD != `C:\work\sample-repo` || start.Branch != "main" || start.Model != "opus" || start.SubscriptionID != "profile-1" {
		t.Errorf("session_start fields = %+v, want provider/cwd/branch/model/subscription_id filled", start)
	}
	if end.Kind != handoff.KindSessionEnd {
		t.Errorf("record 1 kind = %q, want %q", end.Kind, handoff.KindSessionEnd)
	}
	if end.Text != "completed" {
		t.Errorf("session_end text = %q, want %q", end.Text, "completed")
	}
}

// TestHandoffSessionEndFoldsStateAndReasonIntoText covers the "exec_not_found"
// style reason code path (wrapper.classifyStartFailure), which has nowhere
// else in Record to live.
func TestHandoffSessionEndFoldsStateAndReasonIntoText(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 102
	s.recordHandoffSessionEnd(sessionID, "error", "exec_not_found")

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 {
		t.Fatalf("got %d records, want 1", len(got))
	}
	if want := "error: exec_not_found"; got[0].Text != want {
		t.Fatalf("session_end text = %q, want %q", got[0].Text, want)
	}
}

// TestHandoffDoneRecordsClassificationAndFallback is the C2 completion
// criterion for the internal C2 (DONE 要約): a completion produces a
// kind=done line, and the fallback path (git-change only, no marker) must
// remain distinguishable downstream without a new Record field. classifyDoneSummary
// only ever returns "unknown" from the fallback path (fallbackDoneSummaryKind),
// so the "[kind] " prefix on Text doubles as the fallback marker.
func TestHandoffDoneRecordsClassificationAndFallback(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 103
	s.recordHandoffDone(proto.DoneSummary{SessionID: sessionID, Text: "fixed the thing", Kind: "success"})
	s.recordHandoffDone(proto.DoneSummary{SessionID: sessionID, Text: "ターン終了（完了サマリーなし）", Kind: fallbackDoneSummaryKind, Fallback: true})

	got := readHandoffRecords(t, sessionID)
	if len(got) != 2 {
		t.Fatalf("got %d records, want 2: %+v", len(got), got)
	}
	for _, r := range got {
		if r.Kind != handoff.KindDone {
			t.Errorf("record kind = %q, want %q", r.Kind, handoff.KindDone)
		}
	}
	if want := "[success] fixed the thing"; got[0].Text != want {
		t.Errorf("normal done text = %q, want %q", got[0].Text, want)
	}
	if want := "[unknown] ターン終了（完了サマリーなし）"; got[1].Text != want {
		t.Errorf("fallback done text = %q, want %q", got[1].Text, want)
	}
	if !strings.HasPrefix(got[1].Text, "["+fallbackDoneSummaryKind+"]") {
		t.Errorf("fallback row is not distinguishable from a marker-sourced row: %q", got[1].Text)
	}
}

// TestHandoffDoneTextStaysWithinRecordLimit is the explicit 320-rune
// completion criterion, satisfied by handoff.Sanitize even after this
// package's "[kind] " prefix is added on top of done_summary.go's own
// 320-rune truncation.
func TestHandoffDoneTextStaysWithinRecordLimit(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 104
	long := strings.Repeat("あ", handoff.TextMaxRunes)
	s.recordHandoffDone(proto.DoneSummary{SessionID: sessionID, Text: long, Kind: "success"})

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 {
		t.Fatalf("got %d records, want 1", len(got))
	}
	if n := len([]rune(got[0].Text)); n > handoff.TextMaxRunes+1 {
		t.Fatalf("Text = %d runes, want <= %d", n, handoff.TextMaxRunes+1)
	}
}

// TestHandoffGitTurnCarriesPathsAndCommitButNoDiffBody is the C2 completion
// criterion for the internal C3: changed-file paths and the latest commit
// travel, but nothing that ever held a diff body does.
func TestHandoffGitTurnCarriesPathsAndCommitButNoDiffBody(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	gitRoot := initGitTurnTestRepo(t)
	const sessionID = 105
	turn := gitTurnSnapshot{Turn: 3, Files: 2, Added: 5, Removed: 1}
	const secretDiffBody = "+ SECRET_API_KEY=sk-ant-should-never-appear-in-handoff"
	files := []gitShowFile{
		{Status: "M", Path: "tracked.txt", Added: 3, Removed: 1, Diff: secretDiffBody},
		{Status: "A", Path: "new.txt", Added: 2, Removed: 0, Diff: "+ new file"},
	}

	s.recordHandoffGitTurn(sessionID, gitRoot, turn, files)

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 {
		t.Fatalf("got %d records, want 1", len(got))
	}
	r := got[0]
	if r.Kind != handoff.KindGitTurn {
		t.Errorf("kind = %q, want %q", r.Kind, handoff.KindGitTurn)
	}
	if r.Turn != 3 || r.FilesChanged != 2 || r.Added != 5 || r.Removed != 1 {
		t.Errorf("turn fields = %+v, want turn=3 files_changed=2 added=5 removed=1", r)
	}
	if len(r.Files) != 2 || r.Files[0] != "tracked.txt" || r.Files[1] != "new.txt" {
		t.Errorf("files = %v, want [tracked.txt new.txt]", r.Files)
	}
	if r.Commit == "" {
		t.Error("commit hash is empty, want the repo's HEAD")
	}
	if r.CommitSubject != "initial" {
		t.Errorf("commit subject = %q, want %q (from initGitTurnTestRepo)", r.CommitSubject, "initial")
	}

	raw, err := os.ReadFile(mustHandoffPath(t, sessionID))
	if err != nil {
		t.Fatalf("read handoff file: %v", err)
	}
	if strings.Contains(string(raw), secretDiffBody) || strings.Contains(string(raw), "SECRET_API_KEY") {
		t.Fatalf("diff body leaked into handoff file: %s", raw)
	}
}

func mustHandoffPath(t *testing.T, sessionID int) string {
	t.Helper()
	path, err := handoff.PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	return path
}

// TestHandoffLatestCommitMasksSecretInSubject exercises handoffLatestCommit
// directly against a repo whose HEAD subject contains a known secret prefix,
// proving the masking step this package must do itself (handoff.Sanitize
// only ever touches Record.Text, never CommitSubject).
func TestHandoffLatestCommitMasksSecretInSubject(t *testing.T) {
	dir := initGitTurnTestRepo(t)
	run := func(args ...string) {
		t.Helper()
		cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, out)
		}
	}
	if err := os.WriteFile(filepath.Join(dir, "tracked.txt"), []byte("two\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	run("add", "tracked.txt")
	run("-c", "user.name=Test User", "-c", "user.email=test@example.com", "commit", "-m", "token sk-ant-abc123def456ghi789jkl012 leaked")

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_, subject := handoffLatestCommit(ctx, dir)
	if strings.Contains(subject, "sk-ant-abc123def456ghi789jkl012") {
		t.Fatalf("commit subject not masked: %q", subject)
	}
}

// TestHandoffIntentRecordedWhenDoneTextHasNextLine is the C2 completion
// criterion (docs/local/plan_session-handoff-board_c3_intent-layer.md 内部
// C2): a DONE summary carrying the optional "次:"/"未検証:" lines produces a
// kind=intent line in addition to the ordinary kind=done line. Going through
// publishDoneSummaryInternal (not recordHandoffDone directly) matters here —
// the extraction runs on summary.Text after truncateDoneSummary has already
// joined it to one line, which is the shape extractIntentFromDoneText expects.
func TestHandoffIntentRecordedWhenDoneTextHasNextLine(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 107
	s.publishDoneSummaryInternal(proto.DoneSummary{
		SessionID: sessionID,
		Text:      "作業完了。 次: 次のPRをレビューする。 未検証: DBスキーマ変更の影響範囲",
	}, false)

	got := readHandoffRecords(t, sessionID)
	if len(got) != 2 {
		t.Fatalf("got %d records, want 2 (done + intent): %+v", len(got), got)
	}
	if got[0].Kind != handoff.KindDone {
		t.Errorf("record 0 kind = %q, want %q", got[0].Kind, handoff.KindDone)
	}
	intent := got[1]
	if intent.Kind != handoff.KindIntent {
		t.Fatalf("record 1 kind = %q, want %q", intent.Kind, handoff.KindIntent)
	}
	if want := "次: 次のPRをレビューする。 / 未検証: DBスキーマ変更の影響範囲"; intent.Text != want {
		t.Errorf("intent text = %q, want %q", intent.Text, want)
	}
}

// TestHandoffIntentNotRecordedWithoutNextLine is the C2 completion criterion's
// negative case: a DONE summary without either optional label must not grow
// an empty (or any) kind=intent line — the child plan forbids writing an
// empty intent row ("空の行を作らない").
func TestHandoffIntentNotRecordedWithoutNextLine(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 108
	s.publishDoneSummaryInternal(proto.DoneSummary{SessionID: sessionID, Text: "作業完了しました"}, false)

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 {
		t.Fatalf("got %d records, want 1 (done only): %+v", len(got), got)
	}
	if got[0].Kind != handoff.KindDone {
		t.Errorf("record 0 kind = %q, want %q", got[0].Kind, handoff.KindDone)
	}
}

// TestHandoffDisabledWritesNothing proves handoff.enabled: false gates every
// write path in this file through the single appendHandoff/handoffEnabled
// choke point.
func TestHandoffDisabledWritesNothing(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	disabled := false
	s.cfg.Handoff.Enabled = &disabled

	const sessionID = 106
	s.recordHandoffSessionStart(sessionID, "claude", `C:\work\sample-repo`, "main", "opus", "", 0)
	s.recordHandoffSessionEnd(sessionID, "completed", "")
	s.recordHandoffDone(proto.DoneSummary{SessionID: sessionID, Text: "did stuff", Kind: "success"})

	path, err := handoff.PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected no handoff file when disabled, stat err = %v", err)
	}
}

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

// --- 子 plan: plan_derived-session-launch_c4_handoff-routes.md 内部 C1 --------

// newClaudeTranscriptSession registers a live Claude session whose transcript
// file already exists, and returns its path. Everything here is synthetic: a
// throwaway CLAUDE_CONFIG_DIR, a fixture cwd string, and a made-up session
// UUID.
func newClaudeTranscriptSession(t *testing.T, s *Server, id int) string {
	t.Helper()
	const (
		sid = "123e4567-e89b-42d3-a456-426614174000"
		cwd = `C:\work\sample-repo`
	)
	claudeDir := t.TempDir()
	path := writeClaudeTranscript(t, filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd)), sid+".jsonl", cwd, "2026-09-12T10:00:00Z")
	ses := registerTestSession(s, id, "claude")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = "2026-09-12T10:00:00Z"
	ses.ClaudeDir = claudeDir
	ses.AgentSessionID = sid
	s.sessionsMu.Unlock()
	return path
}

// TestHandoffSessionStartRecordsTranscriptPath is the 内部 C1 completion
// criterion on the write side: a Claude session whose transcript is already
// resolvable at registration records its path, and the rendered md points the
// successor at it.
func TestHandoffSessionStartRecordsTranscriptPath(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 201
	want := newClaudeTranscriptSession(t, s, sessionID)
	s.recordHandoffSessionStart(sessionID, "claude", `C:\work\sample-repo`, "main", "opus", "profile-1", 0)

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 || got[0].Transcript != want {
		t.Fatalf("session_start transcript = %+v, want %q", got, want)
	}

	preview, err := s.handoffPreviewFor(sessionID)
	if err != nil {
		t.Fatalf("handoffPreviewFor: %v", err)
	}
	if preview.TranscriptPath != want {
		t.Fatalf("preview.TranscriptPath = %q, want %q", preview.TranscriptPath, want)
	}
	if !strings.Contains(preview.Markdown, want) {
		t.Fatalf("rendered markdown does not carry the transcript path:\n%s", preview.Markdown)
	}
}

// TestHandoffPreviewRecordsTranscriptForLiveSession covers the case the whole
// route exists for: a predecessor that ran out of quota and stopped answering
// has neither ended nor had a transcript file at registration time, so the path
// is only discoverable when a person opens the derive dialog for it.
func TestHandoffPreviewRecordsTranscriptForLiveSession(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 202
	// Board first, with no session registered: nothing is resolvable yet.
	s.recordHandoffSessionStart(sessionID, "claude", `C:\work\sample-repo`, "main", "opus", "", 0)
	if got := readHandoffRecords(t, sessionID); len(got) != 1 || got[0].Transcript != "" {
		t.Fatalf("expected a session_start with no transcript, got %+v", got)
	}

	want := newClaudeTranscriptSession(t, s, sessionID)
	preview, err := s.handoffPreviewFor(sessionID)
	if err != nil {
		t.Fatalf("handoffPreviewFor: %v", err)
	}
	if preview.TranscriptPath != want {
		t.Fatalf("preview.TranscriptPath = %q, want %q", preview.TranscriptPath, want)
	}
	records := readHandoffRecords(t, sessionID)
	if len(records) != 2 || records[1].Kind != handoff.KindTranscript || records[1].Transcript != want {
		t.Fatalf("expected one transcript record appended, got %+v", records)
	}
	// Opening the dialog again must not append a second identical line.
	if _, err := s.handoffPreviewFor(sessionID); err != nil {
		t.Fatalf("handoffPreviewFor (second call): %v", err)
	}
	if got := readHandoffRecords(t, sessionID); len(got) != 2 {
		t.Fatalf("expected the transcript line to be recorded once, got %+v", got)
	}
}

// TestHandoffPreviewOmitsTranscriptPathThatNoLongerExists fixes the read-side
// gate: a path recorded days ago whose file the user has since deleted must not
// be offered to a successor.
func TestHandoffPreviewOmitsTranscriptPathThatNoLongerExists(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 203
	missing := filepath.Join(t.TempDir(), "projects", "gone", "deleted.jsonl")
	if err := handoff.Append(sessionID, handoff.Record{Kind: handoff.KindSessionStart, Provider: "claude", Transcript: missing}); err != nil {
		t.Fatalf("Append: %v", err)
	}

	preview, err := s.handoffIdentityFor(sessionID)
	if err != nil {
		t.Fatalf("handoffIdentityFor: %v", err)
	}
	if preview.TranscriptPath != "" {
		t.Fatalf("preview.TranscriptPath = %q, want empty for a file that is gone", preview.TranscriptPath)
	}
}

// TestHandoffPreviewNeverCreatesABoardForASessionWithout covers the guard in
// ensureHandoffTranscriptRecorded: a live session that has no board (a usage
// probe, or handoff recording turned on mid-session) must not get one from a
// read.
func TestHandoffPreviewNeverCreatesABoardForASessionWithout(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 204
	newClaudeTranscriptSession(t, s, sessionID)
	if _, err := s.handoffPreviewFor(sessionID); err != nil {
		t.Fatalf("handoffPreviewFor: %v", err)
	}

	path, err := handoff.PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected no board to be created by a read, stat err = %v", err)
	}
}

// --- 子 plan: plan_derived-session-launch_c4_handoff-routes.md 内部 C2 --------

// startHandoffNoteAwait puts a session into the state requestHandoffNote leaves
// it in, without going through the PTY injection (which waits on real output
// settling). The memo path is the one the Hub would have chosen.
func startHandoffNoteAwait(t *testing.T, s *Server, id int) string {
	t.Helper()
	path, err := handoff.NotePathFor(id)
	if err != nil {
		t.Fatalf("NotePathFor: %v", err)
	}
	ses := registerTestSession(s, id, "claude")
	s.sessionsMu.Lock()
	ses.handoffNoteAwait = true
	ses.handoffNoteDeadline = time.Now().Add(handoffNoteAwaitTimeout)
	ses.handoffNotePath = path
	ses.handoffNoteSeq++
	s.sessionsMu.Unlock()
	return path
}

// TestHandoffNotePromptNamesPathAndMarker fixes what the predecessor is asked
// to do: write to the Hub-chosen path and print the marker pair on one line.
// A newline in the prompt would be submitted early by some CLIs.
func TestHandoffNotePromptNamesPathAndMarker(t *testing.T) {
	const path = `C:\fake-home\.many-ai-cli\handoff\s7.note.md`
	for _, ja := range []bool{true, false} {
		prompt := handoffNotePrompt(path, ja)
		if !strings.Contains(prompt, path) {
			t.Errorf("prompt (ja=%v) does not name the memo path: %s", ja, prompt)
		}
		if !strings.Contains(prompt, handoffNoteMarkerOpen) || !strings.Contains(prompt, handoffNoteMarkerClose) {
			t.Errorf("prompt (ja=%v) does not name both markers: %s", ja, prompt)
		}
		if strings.ContainsAny(prompt, "\r\n") {
			t.Errorf("prompt (ja=%v) must stay on one line: %q", ja, prompt)
		}
	}
}

// TestHandoffNoteRequestRejectedWhenSessionIsGone covers the case the button is
// most likely to hit: the predecessor already stopped, so there is nobody left
// to write the memo. The caller gets a reason, not a silent success.
func TestHandoffNoteRequestRejectedWhenSessionIsGone(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	if path, reason := s.requestHandoffNote(301); reason != "session_not_writable" || path != "" {
		t.Fatalf("requestHandoffNote = (%q, %q), want ('', session_not_writable)", path, reason)
	}

	disabled := false
	s.cfg.Handoff.Enabled = &disabled
	if _, reason := s.requestHandoffNote(301); reason != "handoff_disabled" {
		t.Fatalf("requestHandoffNote with handoff disabled = %q, want handoff_disabled", reason)
	}
}

// TestHandoffNoteMarkerRecordsThePathOnlyWhenTheFileExists is the 内部 C2
// completion criterion on the record side: the marker is the AI's claim, the
// file's existence is the Hub's own check, and only the path is ever recorded.
func TestHandoffNoteMarkerRecordsThePathOnlyWhenTheFileExists(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 302
	path := startHandoffNoteAwait(t, s, sessionID)

	// The AI claims it wrote the memo, but nothing is there.
	s.handleHandoffNoteChunk(sessionID, handoffNoteMarkerOpen+" written "+handoffNoteMarkerClose+"\n")
	if _, err := os.Stat(mustHandoffPath(t, sessionID)); !os.IsNotExist(err) {
		t.Fatalf("a claim with no file must record nothing, stat err = %v", err)
	}

	// Now the memo really is there.
	path2 := startHandoffNoteAwait(t, s, sessionID)
	if path2 != path {
		t.Fatalf("memo path changed between requests: %q -> %q", path, path2)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("# 引き継ぎメモ\n- 次の一手: なし\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.handleHandoffNoteChunk(sessionID, handoffNoteMarkerOpen+" written "+handoffNoteMarkerClose+"\n")

	got := readHandoffRecords(t, sessionID)
	if len(got) != 1 || got[0].Kind != handoff.KindNote || got[0].Note != path {
		t.Fatalf("records = %+v, want one note record naming %q", got, path)
	}
	if got[0].Text != "" {
		t.Fatalf("the memo's contents must never reach a Record: %+v", got[0])
	}

	preview, err := s.handoffPreviewFor(sessionID)
	if err != nil {
		t.Fatalf("handoffPreviewFor: %v", err)
	}
	if preview.NotePath != path {
		t.Fatalf("preview.NotePath = %q, want %q", preview.NotePath, path)
	}
	if !strings.Contains(preview.Markdown, path) {
		t.Fatalf("rendered markdown does not point at the memo:\n%s", preview.Markdown)
	}
}

// TestHandoffNoteAwaitIsGivenUpOnTimeout fixes the cutoff: a predecessor that
// never answers must not leave the session waiting forever, and the browser is
// told once that no memo was written.
func TestHandoffNoteAwaitIsGivenUpOnTimeout(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()

	const sessionID = 303
	startHandoffNoteAwait(t, s, sessionID)
	s.sessionsMu.Lock()
	seq := s.sessions[sessionID].handoffNoteSeq
	s.sessionsMu.Unlock()

	// A timer from an older request must not cancel this one.
	s.expireHandoffNote(sessionID, seq-1)
	s.sessionsMu.Lock()
	stillAwaiting := s.sessions[sessionID].handoffNoteAwait
	s.sessionsMu.Unlock()
	if !stillAwaiting {
		t.Fatal("a stale generation's timer cancelled the current request")
	}

	s.expireHandoffNote(sessionID, seq)
	s.sessionsMu.Lock()
	stillAwaiting = s.sessions[sessionID].handoffNoteAwait
	s.sessionsMu.Unlock()
	if stillAwaiting {
		t.Fatal("expireHandoffNote left the session waiting")
	}
	if _, err := os.Stat(mustHandoffPath(t, sessionID)); !os.IsNotExist(err) {
		t.Fatalf("a timed-out request must record nothing, stat err = %v", err)
	}
}

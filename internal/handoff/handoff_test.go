package handoff

import (
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"
	"time"
)

func withTempHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return home
}

func TestAppendAndReadAllRoundTrip(t *testing.T) {
	withTempHome(t)

	const sessionID = 42
	want := []Record{
		{Kind: KindSessionStart, Provider: "claude", CWD: `C:\work\sample-repo`},
		{Kind: KindDone, Text: "fixed the thing"},
		{Kind: KindSessionEnd, Text: "done for the day"},
	}
	for _, r := range want {
		if err := Append(sessionID, r); err != nil {
			t.Fatalf("Append: %v", err)
		}
	}

	path, err := PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	got, err := ReadAll(path)
	if err != nil {
		t.Fatalf("ReadAll: %v", err)
	}
	if len(got) != len(want) {
		t.Fatalf("read back %d records, want %d", len(got), len(want))
	}
	for i, r := range got {
		if r.Kind != want[i].Kind {
			t.Errorf("record %d: kind = %q, want %q", i, r.Kind, want[i].Kind)
		}
		if r.Version != RecordVersion {
			t.Errorf("record %d: version = %d, want %d", i, r.Version, RecordVersion)
		}
		if r.SessionID != sessionID {
			t.Errorf("record %d: session id = %d, want %d", i, r.SessionID, sessionID)
		}
		if strings.TrimSpace(r.TS) == "" {
			t.Errorf("record %d: ts is empty", i)
		}
	}
}

// TestReadAllSkipsCorruptLineButKeepsNeighbors is the C1 completion criterion:
// a partially written line (crash mid-write) must not take out the lines
// before or after it.
func TestReadAllSkipsCorruptLineButKeepsNeighbors(t *testing.T) {
	withTempHome(t)

	const sessionID = 7
	if err := Append(sessionID, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append 1: %v", err)
	}
	if err := Append(sessionID, Record{Kind: KindSessionEnd}); err != nil {
		t.Fatalf("Append 2: %v", err)
	}

	path, err := PathFor(sessionID)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	// Splice a truncated (invalid JSON) line between the two good ones.
	orig, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read file: %v", err)
	}
	lines := strings.Split(strings.TrimRight(string(orig), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 lines before corruption, got %d", len(lines))
	}
	corrupted := lines[0] + "\n" + `{"version":1,"kind":"git_tur` + "\n" + lines[1] + "\n"
	if err := os.WriteFile(path, []byte(corrupted), 0o600); err != nil {
		t.Fatalf("write corrupted file: %v", err)
	}

	got, err := ReadAll(path)
	if err != nil {
		t.Fatalf("ReadAll: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("got %d records, want 2 (corrupt line should be skipped, not the file)", len(got))
	}
	if got[0].Kind != KindSessionStart || got[1].Kind != KindSessionEnd {
		t.Fatalf("unexpected records around the corrupt line: %+v", got)
	}
}

// allowedRecordFields is the C1 completion criterion: the record type must
// have no field a PTY transcript, a file's contents, a diff, or an
// environment variable could be put into. Extending this list is a design
// decision (read the 親 plan の不変条件 1 first), not a routine test fix.
var allowedRecordFields = map[string]bool{
	"Version":        true,
	"TS":             true,
	"SessionID":      true,
	"Provider":       true,
	"CWD":            true,
	"Branch":         true,
	"Model":          true,
	"SubscriptionID": true,
	"Kind":           true,
	"Files":          true,
	"Commit":         true,
	"CommitSubject":  true,
	"Added":          true,
	"Removed":        true,
	"FilesChanged":   true,
	"Turn":           true,
	"WorkDoc":        true,
	"Text":           true,
	"HandoffFrom":    true,
	// Transcript / Note were added on 2026-09-12 as a deliberate widening
	// (子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md
	// 内部 C1・C2, 親 plan 不変条件 2): a **path** may be recorded, its contents
	// may not. Both name a file the successor opens with its own tools; neither
	// is ever read by many-ai-cli. A field holding transcript text, a diff, or a
	// memo's body still belongs nowhere on this type.
	"Transcript": true,
	"Note":       true,
}

func TestRecordFieldsAreTheAllowlist(t *testing.T) {
	typ := reflect.TypeOf(Record{})
	seen := map[string]bool{}
	for i := 0; i < typ.NumField(); i++ {
		name := typ.Field(i).Name
		seen[name] = true
		if !allowedRecordFields[name] {
			t.Errorf("Record gained an unexpected field %q. "+
				"Check 親 plan (docs/local/plan_session-handoff-board.md) 方針 不変条件 1 before "+
				"widening the allowlist in handoff.go and handoff_test.go.", name)
		}
	}
	for name := range allowedRecordFields {
		if !seen[name] {
			t.Errorf("allowlist names field %q which no longer exists on Record; trim handoff_test.go", name)
		}
	}
}

func TestSanitizeMasksSecretsInText(t *testing.T) {
	r := Sanitize(Record{Kind: KindDone, Text: "token sk-ant-abc123def456ghi789jkl012 leaked"})
	if strings.Contains(r.Text, "sk-ant-abc123def456ghi789jkl012") {
		t.Fatalf("Sanitize did not mask a known secret prefix: %q", r.Text)
	}
}

func TestSanitizeTruncatesText(t *testing.T) {
	long := strings.Repeat("a", TextMaxRunes+50)
	r := Sanitize(Record{Kind: KindDone, Text: long})
	if got := len([]rune(r.Text)); got > TextMaxRunes+1 { // +1 for the "…" marker rune
		t.Fatalf("Text not truncated: got %d runes, want <= %d", got, TextMaxRunes+1)
	}
	if !strings.HasSuffix(r.Text, "…") {
		t.Fatalf("truncated Text missing ellipsis marker: %q", r.Text)
	}
}

func TestSanitizeCapsFilesCount(t *testing.T) {
	files := make([]string, FilesMaxCount+5)
	for i := range files {
		files[i] = filepath.Join("dir", "file.go")
	}
	r := Sanitize(Record{Kind: KindGitTurn, Files: files})
	if len(r.Files) != FilesMaxCount+1 {
		t.Fatalf("Files length = %d, want %d (kept + summary entry)", len(r.Files), FilesMaxCount+1)
	}
	last := r.Files[len(r.Files)-1]
	if !strings.Contains(last, "5") {
		t.Fatalf("summary entry should mention the 5 overflow files: %q", last)
	}
}

func TestAppendCreatesPrivateDirAndFile(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows does not report POSIX mode bits reliably")
	}
	withTempHome(t)

	if err := Append(1, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append: %v", err)
	}
	dir, err := Dir()
	if err != nil {
		t.Fatalf("Dir: %v", err)
	}
	dirInfo, err := os.Stat(dir)
	if err != nil {
		t.Fatalf("stat dir: %v", err)
	}
	if got := dirInfo.Mode().Perm(); got != 0o700 {
		t.Fatalf("dir mode = %#o, want %#o", got, 0o700)
	}
	path, err := PathFor(1)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	fileInfo, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat file: %v", err)
	}
	if got := fileInfo.Mode().Perm(); got != 0o600 {
		t.Fatalf("file mode = %#o, want %#o", got, 0o600)
	}
}

func TestPruneOlderThanRemovesOnlyStaleFiles(t *testing.T) {
	withTempHome(t)

	if err := Append(1, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append old: %v", err)
	}
	if err := Append(2, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append fresh: %v", err)
	}

	oldPath, err := PathFor(1)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	old := time.Now().Add(-30 * 24 * time.Hour)
	if err := os.Chtimes(oldPath, old, old); err != nil {
		t.Fatalf("Chtimes: %v", err)
	}

	cutoff := time.Now().Add(-14 * 24 * time.Hour)
	if err := PruneOlderThan(cutoff); err != nil {
		t.Fatalf("PruneOlderThan: %v", err)
	}

	if _, err := os.Stat(oldPath); !os.IsNotExist(err) {
		t.Fatalf("expected stale handoff file to be removed, stat err = %v", err)
	}
	freshPath, err := PathFor(2)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	if _, err := os.Stat(freshPath); err != nil {
		t.Fatalf("fresh handoff file should survive prune: %v", err)
	}
}

// TestPruneOlderThanRemovesStaleNoteFiles is the 内部 C2 completion criterion
// for retention (子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md):
// the memo an AI wrote is reclaimed by the same 14-day sweep as the board it is
// named from, so a memo cannot outlive the record that points at it.
func TestPruneOlderThanRemovesStaleNoteFiles(t *testing.T) {
	withTempHome(t)

	if err := Append(1, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append: %v", err)
	}
	notePath, err := NotePathFor(1)
	if err != nil {
		t.Fatalf("NotePathFor: %v", err)
	}
	if err := os.WriteFile(notePath, []byte("# memo\n"), 0o600); err != nil {
		t.Fatalf("write note: %v", err)
	}
	old := time.Now().Add(-30 * 24 * time.Hour)
	if err := os.Chtimes(notePath, old, old); err != nil {
		t.Fatalf("Chtimes: %v", err)
	}

	if err := PruneOlderThan(time.Now().Add(-14 * 24 * time.Hour)); err != nil {
		t.Fatalf("PruneOlderThan: %v", err)
	}
	if _, err := os.Stat(notePath); !os.IsNotExist(err) {
		t.Fatalf("expected the stale memo to be removed, stat err = %v", err)
	}
}

func TestStatDirReportsFileCount(t *testing.T) {
	withTempHome(t)

	status, err := StatDir()
	if err != nil {
		t.Fatalf("StatDir (missing dir): %v", err)
	}
	if status.Exists || status.Files != 0 {
		t.Fatalf("expected empty status before any writes, got %+v", status)
	}

	if err := Append(1, Record{Kind: KindSessionStart}); err != nil {
		t.Fatalf("Append: %v", err)
	}
	status, err = StatDir()
	if err != nil {
		t.Fatalf("StatDir: %v", err)
	}
	if !status.Exists || status.Files != 1 {
		t.Fatalf("expected 1 file after one Append, got %+v", status)
	}
}

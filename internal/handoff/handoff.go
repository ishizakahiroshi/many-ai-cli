// Package handoff records what a session did, in a form deliberately narrow
// enough that a different AI CLI can safely read it after this one stops.
//
// Design rule (親 plan: docs/local/plan_session-handoff-board.md の「方針」節
// 不変条件 1・2. 子 plan: docs/local/plan_session-handoff-board_c1_record-store.md):
//
// What is allowed into a Record is decided by the Go type, not by scanning
// free text for secrets afterward. Record has no field a PTY transcript, a
// file's contents, a diff, an environment variable, or a user's raw input
// could be put into — only Commit/CommitSubject/Files (already public in
// `git log --stat`), the session's own metadata, and one free-text Text field
// are representable at all. sessionlog.MaskSecrets is a denylist of known
// vendor key prefixes and names; it does not catch a bare high-entropy
// string, a customer name typed into a commit subject, or a hostname in a
// path. Masking only ever runs over Text (Sanitize), because Text is the one
// field an AI can put anything into. Widening this type is a decision, not a
// bug fix — read the plan's 不変条件 1 before adding a field, and add the new
// field name to the allowlist in handoff_test.go or the guard test will fail
// on purpose.
//
// This mirrors internal/subscription/adapter.go and internal/doctor/residue.go:
// the rule lives next to the code that would break it, not in CLAUDE.md.
package handoff

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
)

// RecordVersion is stamped into every record written by this build.
//
// It is a per-line field, not a file-level one, on purpose: a jsonl file
// accumulates lines across many `many-ai-cli` versions over its 14-day
// retention window, and a reader must keep understanding old lines after the
// shape changes. Bump this when Record's meaning changes in a way a reader
// needs to branch on; do not bump it just because a field was added.
const RecordVersion = 1

// Record kinds. Kept as plain strings (not a Go type with a Valid() method)
// because this package never rejects an unknown Kind on write — a future
// build's new Kind must still append cleanly to a file an older build reads.
const (
	KindSessionStart = "session_start"
	KindSessionEnd   = "session_end"
	KindDone         = "done"
	KindGitTurn      = "git_turn"
	KindIntent       = "intent"
	KindTurnSummary  = "turn_summary"
)

// Record is one line of a session's handoff log
// (~/.many-ai-cli/handoff/s<id>.jsonl). See the package doc for the allowlist
// rule this type enforces.
type Record struct {
	Version   int    `json:"version"`
	TS        string `json:"ts"`
	SessionID int    `json:"session_id"`
	Provider  string `json:"provider,omitempty"`
	CWD       string `json:"cwd,omitempty"`
	Branch    string `json:"branch,omitempty"`
	Model     string `json:"model,omitempty"`
	// SubscriptionID is the profile ID only (never a credential or display
	// name), matching sessionstore.SessionStart.SubscriptionID.
	SubscriptionID string `json:"subscription_id,omitempty"`
	Kind           string `json:"kind"`
	// Files holds changed-file paths only, never file contents or diffs.
	Files         []string `json:"files,omitempty"`
	Commit        string   `json:"commit,omitempty"`
	CommitSubject string   `json:"commit_subject,omitempty"`
	Added         int      `json:"added,omitempty"`
	Removed       int      `json:"removed,omitempty"`
	FilesChanged  int      `json:"files_changed,omitempty"`
	Turn          int      `json:"turn,omitempty"`
	// WorkDoc is the plan/bugfix md path the session had open, not its content.
	WorkDoc string `json:"work_doc,omitempty"`
	// Text is the only free-form field: a DONE summary or a one-line
	// AI-written note. Sanitize runs sessionlog.MaskSecrets and a length cap
	// over this field before Append writes it.
	Text string `json:"text,omitempty"`
	// HandoffFrom is the predecessor session's ID, set only on the successor's
	// own KindSessionStart record when it was spawned from a handoff (子 plan:
	// docs/local/plan_session-handoff-board_c5_handoff-md.md 内部 C3
	// 「起動した新しいセッションの看板の session_start に『引き継ぎ元のセッ
	// ション』を持たせる」). This is a bare int identifier — the same shape as
	// SessionID — not free text, so it does not widen what an AI or a PTY can
	// put into a Record; it just names another session the same way SessionID
	// names this one.
	HandoffFrom int `json:"handoff_from,omitempty"`
}

const (
	// TextMaxRunes mirrors internal/hub/done_summary.go's doneSummaryMaxRunes
	// (320), because a DONE summary is what usually lands in Text verbatim.
	// Not imported directly: duplicating one small constant here beats adding
	// an import edge between hub and handoff for a single number.
	TextMaxRunes = 320
	// FilesMaxCount bounds how many changed-file paths one record keeps.
	// Over the limit, the first FilesMaxCount paths are kept and a single
	// summary entry ("…他 N 件") replaces the rest.
	FilesMaxCount = 20
)

// Sanitize applies the "last net" (親 plan 不変条件 2): MaskSecrets over Text,
// plus length/count limits on Text and Files. The other fields are already
// closed by the type (allowlist), so masking never touches them. Append calls
// this; callers that build a Record for a test or a preview may call it
// directly too.
func Sanitize(r Record) Record {
	r.Text = truncateRunes(sessionlog.MaskSecrets(r.Text), TextMaxRunes)
	r.Files = truncateFiles(r.Files, FilesMaxCount)
	return r
}

func truncateRunes(s string, max int) string {
	if max <= 0 {
		return s
	}
	runes := []rune(s)
	if len(runes) <= max {
		return s
	}
	return string(runes[:max]) + "…"
}

func truncateFiles(files []string, max int) []string {
	if max <= 0 || len(files) <= max {
		return files
	}
	kept := make([]string, 0, max+1)
	kept = append(kept, files[:max]...)
	kept = append(kept, fmt.Sprintf("…他 %d 件", len(files)-max))
	return kept
}

// Dir returns ~/.many-ai-cli/handoff.
func Dir() (string, error) {
	base, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(base, "handoff"), nil
}

// PathFor returns the jsonl path for one session's handoff log.
func PathFor(sessionID int) (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, fmt.Sprintf("s%d.jsonl", sessionID)), nil
}

// RenderedPathFor returns the path of the rendered handoff markdown for a
// session (子 plan 内部 C1 「出力先」). It sits next to the jsonl in the same
// directory so PruneOlderThan's single sweep reclaims both.
func RenderedPathFor(sessionID int) (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, fmt.Sprintf("s%d_handoff.md", sessionID)), nil
}

// ReadSession is a small convenience wrapper over PathFor+ReadAll for the
// common case of reading one session's own records. A missing jsonl file is
// reported as (nil, nil) — a session with handoff recording enabled but no
// records yet (or a session ID nothing was ever written for) is not an error
// for a caller that just wants to know "is there anything to render".
func ReadSession(sessionID int) ([]Record, error) {
	path, err := PathFor(sessionID)
	if err != nil {
		return nil, err
	}
	if _, statErr := os.Stat(path); statErr != nil {
		if os.IsNotExist(statErr) {
			return nil, nil
		}
		return nil, statErr
	}
	return ReadAll(path)
}

// WriteRendered renders the current markdown for a session and writes it to
// RenderedPathFor, returning the path and the rendered text. It is safe to
// call repeatedly (each call overwrites the previous rendering with a fresh
// one from the latest records) — the file on disk is a cache of the last
// preview shown, not a separate source of truth from the jsonl.
func WriteRendered(sessionID int) (path, markdown string, err error) {
	records, err := ReadSession(sessionID)
	if err != nil {
		return "", "", err
	}
	markdown = RenderMarkdown(sessionID, records)
	path, err = RenderedPathFor(sessionID)
	if err != nil {
		return "", "", err
	}
	if err := os.MkdirAll(filepath.Dir(path), sessionlog.PrivateDirMode); err != nil {
		return "", "", fmt.Errorf("create handoff dir: %w", err)
	}
	if err := os.WriteFile(path, []byte(markdown), sessionlog.PrivateFileMode); err != nil {
		return "", "", fmt.Errorf("write rendered handoff: %w", err)
	}
	return path, markdown, nil
}

// appendMu serializes every Append call in this process. Handoff writes are
// not on a hot path (session lifecycle, DONE summaries, per-turn git diffs —
// not PTY data), so one process-wide mutex is simpler than a per-file mutex
// map that would otherwise grow for the life of the Hub process. It is
// package-private and must never be held across a caller's own lock (callers
// must not call Append while holding a Hub lock, and this package never
// acquires one).
var appendMu sync.Mutex

// Append writes one record as a JSON line to the session's handoff file,
// creating the directory (0700) and file (0600) if needed. A write failure is
// returned to the caller to log as a warning; recording is not the session's
// primary job, so callers must not treat a failure here as fatal.
func Append(sessionID int, r Record) error {
	r.Version = RecordVersion
	if r.SessionID == 0 {
		r.SessionID = sessionID
	}
	if strings.TrimSpace(r.TS) == "" {
		r.TS = time.Now().Format(time.RFC3339)
	}
	r = Sanitize(r)
	line, err := json.Marshal(r)
	if err != nil {
		return fmt.Errorf("marshal handoff record: %w", err)
	}
	path, err := PathFor(sessionID)
	if err != nil {
		return fmt.Errorf("resolve handoff path: %w", err)
	}

	appendMu.Lock()
	defer appendMu.Unlock()

	if err := os.MkdirAll(filepath.Dir(path), sessionlog.PrivateDirMode); err != nil {
		return fmt.Errorf("create handoff dir: %w", err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, sessionlog.PrivateFileMode)
	if err != nil {
		return fmt.Errorf("open handoff file: %w", err)
	}
	defer f.Close()
	if _, err := f.Write(append(line, '\n')); err != nil {
		return fmt.Errorf("write handoff record: %w", err)
	}
	return nil
}

// ReadAll reads every well-formed record from a handoff jsonl file, in file
// order. A line that fails to parse (for example a partial write cut off by a
// crash) is skipped; it does not stop the lines before or after it from being
// read.
func ReadAll(path string) ([]Record, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	var out []Record
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		var r Record
		if err := json.Unmarshal([]byte(line), &r); err != nil {
			continue
		}
		out = append(out, r)
	}
	if err := scanner.Err(); err != nil {
		return out, err
	}
	return out, nil
}

// PruneOlderThan removes handoff files under Dir() whose modification time is
// before cutoff. It is a plain directory sweep — no attempt to catch every
// process kill sooner. The contract is "a file older than retention
// disappears the next time this runs" (Hub startup or its periodic
// maintenance loop), which matches internal/doctor/residue.go's rule that a
// leftover a killed process couldn't clean up must be reclaimed on the next
// start rather than chased with more shutdown-path complexity.
func PruneOlderThan(cutoff time.Time) error {
	dir, err := Dir()
	if err != nil {
		return err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	for _, e := range entries {
		if e.IsDir() || !isHandoffFileName(e.Name()) {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
	return nil
}

// isHandoffFileName reports whether name is a file this package owns inside
// Dir(): either a session's jsonl log or its rendered markdown preview
// (RenderedPathFor). Both share the same 14-day retention sweep (子 plan
// 内部 C1 「C1（器）の保持 14 日と同じ掃除に乗せる」).
func isHandoffFileName(name string) bool {
	return strings.HasSuffix(name, ".jsonl") || strings.HasSuffix(name, "_handoff.md")
}

// DirStatus summarizes the handoff directory for `many-ai-cli doctor`.
type DirStatus struct {
	Exists    bool
	Files     int
	OldestAge time.Duration
}

// StatDir reports how many handoff files exist and the age of the oldest one.
// A missing directory is not an error (Exists stays false).
func StatDir() (DirStatus, error) {
	dir, err := Dir()
	if err != nil {
		return DirStatus{}, err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return DirStatus{}, nil
		}
		return DirStatus{}, err
	}
	var status DirStatus
	var oldest time.Time
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		status.Files++
		if oldest.IsZero() || info.ModTime().Before(oldest) {
			oldest = info.ModTime()
		}
	}
	status.Exists = status.Files > 0
	if !oldest.IsZero() {
		status.OldestAge = time.Since(oldest)
	}
	return status, nil
}

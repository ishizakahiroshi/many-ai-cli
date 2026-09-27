package hub

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// A child the user stopped arrives as a task-notification with status
// "killed". It must resolve as terminal (failed); treating it as a launch
// signal kept the child running forever and froze the turn cutoff.
func TestClaudeSubagentClassifyStatusTreatsUnsuccessfulEndsAsTerminal(t *testing.T) {
	cases := []struct {
		status       string
		done, failed bool
	}{
		{"completed", true, false},
		{"failed", true, true},
		{"killed", true, true},
		{"Killed", true, true},
		{"stopped", true, true},
		{"cancelled", true, true},
		{"canceled", true, true},
		{"aborted", true, true},
		{"interrupted", true, true},
		{"async_launched", false, false},
		{"", false, false},
	}
	for _, c := range cases {
		done, failed := claudeSubagentClassifyStatus(c.status)
		if done != c.done || failed != c.failed {
			t.Errorf("claudeSubagentClassifyStatus(%q) = (%v, %v), want (%v, %v)", c.status, done, failed, c.done, c.failed)
		}
	}
}

// When the session's identifying fields change (a new AgentSessionID after
// /clear, NativeLogPath set later by Codex's Stop hook), the cached parent path
// must not keep pointing at the old transcript just because that file still
// exists. The re-resolve must happen right away, not after the 30s retry window.
func TestRunSubagentTreePollReresolvesWhenResolverInputsChange(t *testing.T) {
	tmp := t.TempDir()
	oldPath := filepath.Join(tmp, "old.jsonl")
	newPath := filepath.Join(tmp, "new.jsonl")
	for _, p := range []string{oldPath, newPath} {
		if err := os.WriteFile(p, []byte("{}"), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = true
	ses := registerTestSession(s, 1, "claude")
	s.sessionsMu.Lock()
	ses.AgentSessionID = "session-a"
	s.sessionsMu.Unlock()

	var resolveCalls int
	var readPaths []string
	savedResolver := subagentPathResolverByKey["subagent:claude-v1"]
	subagentPathResolverByKey["subagent:claude-v1"] = func(info subagentParentInfo) (string, bool) {
		resolveCalls++
		if info.AgentSessionID == "session-b" {
			return newPath, true
		}
		return oldPath, true
	}
	savedReader := subagentReaderByKey["subagent:claude-v1"]
	subagentReaderByKey["subagent:claude-v1"] = func(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
		readPaths = append(readPaths, transcriptPath)
		return nil, prior, nil
	}
	defer func() {
		subagentPathResolverByKey["subagent:claude-v1"] = savedResolver
		subagentReaderByKey["subagent:claude-v1"] = savedReader
	}()

	stopTimer := func() {
		s.sessionsMu.Lock()
		if ses.subagentTimer != nil {
			ses.subagentTimer.Stop()
			ses.subagentTimer = nil
		}
		s.sessionsMu.Unlock()
	}

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 1 {
		t.Fatalf("resolveCalls with unchanged inputs = %d, want 1 (cached)", resolveCalls)
	}

	// Same poll window (well inside 30s), old file still exists, but the
	// session now identifies a different transcript.
	s.sessionsMu.Lock()
	ses.AgentSessionID = "session-b"
	s.sessionsMu.Unlock()

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 2 {
		t.Fatalf("resolveCalls right after the inputs changed = %d, want 2 (immediate re-resolve)", resolveCalls)
	}
	if got := readPaths[len(readPaths)-1]; got != newPath {
		t.Fatalf("reader read %q after the inputs changed, want the newly resolved %q", got, newPath)
	}
	s.sessionsMu.Lock()
	gotPath := ses.subagentParentPath
	s.sessionsMu.Unlock()
	if gotPath != newPath {
		t.Fatalf("subagentParentPath = %q, want %q", gotPath, newPath)
	}

	// And the new resolution is cached again.
	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 2 {
		t.Fatalf("resolveCalls after re-caching = %d, want still 2", resolveCalls)
	}
}

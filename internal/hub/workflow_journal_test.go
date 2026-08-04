package hub

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

func TestWorkflowJournalEventPrivacyShape(t *testing.T) {
	typ := reflect.TypeOf(workflowJournalEvent{})
	if typ.NumField() != 2 || typ.Field(0).Name != "Type" || typ.Field(1).Name != "AgentID" {
		t.Fatalf("journal decode shape must contain only Type and AgentID: %v", typ)
	}
}

func TestWorkflowJournalTailLargeLineAndRestartIdempotence(t *testing.T) {
	path := filepath.Join(t.TempDir(), "journal.jsonl")
	body := strings.Repeat("synthetic-result-body-", 5000) // comfortably above 64 KiB
	data := "{\"type\":\"started\",\"agentId\":\"a1\"}\n" +
		"{\"type\":\"result\",\"agentId\":\"a1\",\"result\":\"" + body + "\"}\n"
	if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := tailWorkflowJournal(path, workflowJournalFileState{})
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Started) != 1 || len(state.Results) != 1 || state.Offset != int64(len(data)) {
		t.Fatalf("large-line tail mismatch: started=%d done=%d offset=%d", len(state.Started), len(state.Results), state.Offset)
	}
	restarted, err := tailWorkflowJournal(path, state)
	if err != nil {
		t.Fatal(err)
	}
	if len(restarted.Started) != 1 || len(restarted.Results) != 1 || restarted.Offset != state.Offset {
		t.Fatalf("restart double-counted journal: %+v", restarted)
	}
}

func TestWorkflowJournalTailCarriesIncompleteLine(t *testing.T) {
	path := filepath.Join(t.TempDir(), "journal.jsonl")
	partial := "{\"type\":\"started\",\"agentId\":\"a1\"}"
	if err := os.WriteFile(path, []byte(partial), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := tailWorkflowJournal(path, workflowJournalFileState{})
	if err != nil {
		t.Fatal(err)
	}
	if state.Offset != 0 || len(state.Started) != 0 {
		t.Fatalf("incomplete line was consumed: %+v", state)
	}
	if err := os.WriteFile(path, []byte(partial+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err = tailWorkflowJournal(path, state)
	if err != nil {
		t.Fatal(err)
	}
	if state.Offset != int64(len(partial)+1) || len(state.Started) != 1 {
		t.Fatalf("completed line was not consumed: %+v", state)
	}
}

func TestWorkflowJournalTailFreezesOnTruncate(t *testing.T) {
	path := filepath.Join(t.TempDir(), "journal.jsonl")
	data := "{\"type\":\"started\",\"agentId\":\"a1\"}\n{\"type\":\"result\",\"agentId\":\"a1\"}\n"
	if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := tailWorkflowJournal(path, workflowJournalFileState{})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("{}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	frozen, err := tailWorkflowJournal(path, state)
	if err != nil {
		t.Fatal(err)
	}
	if !frozen.Frozen || len(frozen.Started) != 1 || len(frozen.Results) != 1 || frozen.Offset != state.Offset {
		t.Fatalf("truncate did not freeze trusted counts: %+v", frozen)
	}
}

func TestWorkflowJournalAssociationRejectsAmbiguousSession(t *testing.T) {
	groups := []workflowJournalGroup{
		{SessionDir: "one", Started: 3, Done: 1},
		{SessionDir: "two", Started: 4, Done: 2},
	}
	if _, ok := selectWorkflowJournalGroup(groups, 2); ok {
		t.Fatal("ambiguous same-cwd journal was associated")
	}
	group, ok := selectWorkflowJournalGroup(groups, 4)
	if !ok || group.SessionDir != "two" {
		t.Fatalf("unique range association failed: %+v %v", group, ok)
	}
}

func TestWorkflowJournalDiscoveryExcludesPastCompletedRun(t *testing.T) {
	projectDir := t.TempDir()
	journalDir := filepath.Join(projectDir, "old-session", "subagents", "workflows", "wf_old")
	if err := os.MkdirAll(journalDir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(journalDir, "journal.jsonl")
	data := "{\"type\":\"started\",\"agentId\":\"a1\"}\n{\"type\":\"result\",\"agentId\":\"a1\"}\n"
	if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
		t.Fatal(err)
	}
	detectedAt := time.Now()
	old := detectedAt.Add(-time.Second)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatal(err)
	}
	groups, err := discoverWorkflowJournalGroups(projectDir, detectedAt)
	if err != nil {
		t.Fatal(err)
	}
	if len(groups) != 0 {
		t.Fatalf("past completed run was included: %+v", groups)
	}
}

func TestWorkflowJournalSettleAndNotificationAreOneShot(t *testing.T) {
	now := time.Now()
	cfg := &config.Config{}
	cfg.Workflow.JournalEnabled = true
	ses := &session{
		Provider:                  "claude",
		workflowVTHasSignal:       true,
		workflowJournalGeneration: 1,
		workflowVTProgress: &proto.WorkflowProgress{
			Detected: true, Source: "vt-summary", Done: 2, Total: 2, Settled: true, SettledBy: "vt",
		},
		vtCounts: workflowCounts{Detected: true, Done: 2, Total: 2, Settled: true, SettledBy: "vt"},
	}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}
	files := map[string]workflowJournalFileState{"synthetic": {
		Offset:  10,
		Started: map[string]struct{}{"a1": {}, "a2": {}},
		Results: map[string]struct{}{"a1": {}, "a2": {}},
		ModTime: now.Add(-workflowJournalSettleDelay - time.Second),
	}}
	s.applyWorkflowJournalPoll(1, ses, 1, files, "synthetic-session", true, now)
	if !ses.journalCounts.Settled || ses.journalCounts.SettledBy != "journal" || !ses.workflowCompletionNotified {
		t.Fatalf("journal did not settle once: %+v", ses.journalCounts)
	}
	_, _, notifyAgain := s.workflowProgressForBroadcastLocked(ses, now.Add(time.Second))
	if notifyAgain {
		t.Fatal("settled workflow generated a duplicate completion notification")
	}
}

func TestWorkflowJournalTimeoutSettle(t *testing.T) {
	now := time.Now()
	cfg := &config.Config{}
	cfg.Workflow.JournalEnabled = true
	ses := &session{Provider: "claude", workflowJournalGeneration: 1}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}
	files := map[string]workflowJournalFileState{"synthetic": {
		Started: map[string]struct{}{"a1": {}, "a2": {}},
		Results: map[string]struct{}{"a1": {}},
		ModTime: now.Add(-workflowJournalTimeout - time.Second),
	}}
	s.applyWorkflowJournalPoll(1, ses, 1, files, "synthetic-session", true, now)
	if !ses.journalCounts.Settled || ses.journalCounts.SettledBy != "timeout" {
		t.Fatalf("stale incomplete journal did not timeout-settle: %+v", ses.journalCounts)
	}
}

func TestWorkflowJournalStopsPollingAfterIdleMinute(t *testing.T) {
	now := time.Now()
	cfg := &config.Config{}
	cfg.Workflow.JournalEnabled = true
	ses := &session{
		Provider:                  "claude",
		Activity:                  SessionActivity{OutputIdle: true},
		workflowJournalGeneration: 1,
		workflowJournalDetectedAt: now.Add(-workflowJournalIdleStop - time.Second),
		workflowJournalRunning:    true,
	}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}
	s.applyWorkflowJournalPoll(1, ses, 1, nil, "", false, now)
	if ses.workflowJournalRunning || !ses.workflowJournalDormant || ses.workflowJournalTimer != nil {
		t.Fatalf("idle unlinked watcher did not stop: running=%v dormant=%v timer=%v", ses.workflowJournalRunning, ses.workflowJournalDormant, ses.workflowJournalTimer != nil)
	}
}

func TestWorkflowJournalDormantIgnoresVTHeartbeatAndResumesOnOutput(t *testing.T) {
	now := time.Now()
	cfg := &config.Config{}
	cfg.Workflow.JournalEnabled = true
	p := &proto.WorkflowProgress{Detected: true, Source: "vt-summary", Done: 1, Total: 2, Running: 1}
	sig := workflowProgressSignature(p)
	ses := &session{
		Provider:                          "claude",
		Activity:                          SessionActivity{OutputIdle: true},
		workflowVTProgress:                cloneWorkflowProgress(p),
		workflowVTSignature:               sig,
		vtCounts:                          workflowCountsFromProgress(p, now),
		journalCounts:                     workflowCounts{Detected: true, Started: 2, Done: 1},
		workflowJournalFiles:              map[string]workflowJournalFileState{"synthetic": {}},
		workflowJournalSessionDir:         "synthetic-session",
		workflowJournalDetectedAt:         now.Add(-time.Minute),
		workflowJournalRunning:            false,
		workflowJournalDormant:            true,
		workflowJournalDormantVTSignature: sig,
	}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}

	reservedDue := now.Add(2 * time.Hour)
	s.sessionsMu.Lock()
	s.scheduleWorkflowJournalLocked(1, ses, reservedDue)
	reservedTimer := ses.workflowJournalTimer
	reservedGeneration := ses.workflowJournalGeneration
	s.sessionsMu.Unlock()

	// This is the C2 heartbeat path: the same retained VT frame must not wake
	// the dormant journal poller or replace the timeout reservation.
	s.applyWorkflowVTScan(1, ses, cloneWorkflowProgress(p), now.Add(workflowVTHeartbeat))
	stopWorkflowTestTimer(s, ses)
	s.sessionsMu.Lock()
	if ses.workflowJournalRunning || !ses.workflowJournalDormant || ses.workflowJournalTimer != reservedTimer || ses.workflowJournalGeneration != reservedGeneration {
		s.sessionsMu.Unlock()
		reservedTimer.Stop()
		t.Fatalf("VT heartbeat woke dormant watcher: running=%v dormant=%v timer_preserved=%v generation=%d/%d",
			ses.workflowJournalRunning, ses.workflowJournalDormant, ses.workflowJournalTimer == reservedTimer,
			ses.workflowJournalGeneration, reservedGeneration)
	}

	// markRunning clears OutputIdle before a real PTY-triggered scan. That must
	// resume tailing even though the VT signature still belongs to the same run.
	ses.Activity.OutputIdle = false
	s.startWorkflowJournalLocked(1, ses, now.Add(time.Hour))
	resumed := ses.workflowJournalRunning && !ses.workflowJournalDormant && ses.workflowJournalTimer != nil && ses.workflowJournalTimer != reservedTimer
	if ses.workflowJournalTimer != nil {
		ses.workflowJournalTimer.Stop()
		ses.workflowJournalTimer = nil
	}
	s.sessionsMu.Unlock()
	if !resumed {
		t.Fatal("real PTY output did not resume dormant journal tailing")
	}
}

func TestWorkflowJournalToggleOffStopsWatcherAndPush(t *testing.T) {
	cfg := &config.Config{}
	ses := &session{Provider: "claude"}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}
	if s.workflowJournalEnabled() || s.workflowCompletionPushEnabled() {
		t.Fatal("workflow journal or completion Push unexpectedly enabled")
	}
	s.applyWorkflowVTScan(1, ses, &proto.WorkflowProgress{Detected: true, Source: "vt-summary", Done: 1, Total: 2}, time.Now())
	stopWorkflowTestTimer(s, ses)
	if ses.workflowJournalTimer != nil || ses.workflowJournalRunning || ses.journalCounts.Detected {
		t.Fatal("journal watcher started while toggle was off")
	}
}

func TestWorkflowJournalDisabledStillCompletesFromVT(t *testing.T) {
	cfg := &config.Config{}
	cfg.UserPrefs.WorkflowCompletionNotify.Enabled = true
	ses := &session{Provider: "claude"}
	s := &Server{cfg: cfg, sessions: map[int]*session{1: ses}}
	s.applyWorkflowVTScan(1, ses, &proto.WorkflowProgress{Detected: true, Source: "vt-summary", Done: 2, Total: 2}, time.Now())
	stopWorkflowTestTimer(s, ses)
	if ses.journalCounts.Detected || !ses.workflowCompletionNotified {
		t.Fatalf("VT-only completion did not reach common notification transition: journal=%+v notified=%v", ses.journalCounts, ses.workflowCompletionNotified)
	}
}

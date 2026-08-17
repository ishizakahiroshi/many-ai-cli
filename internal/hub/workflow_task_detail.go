package hub

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

// This file implements the internal C1/C2/C3 work described in
// docs/local/plan_workflow-progress-agent-transcript-detail_c2_hub-implementation.md:
//
//   - internal C1: resolve the Claude Code task ID for the Workflow run linked
//     by workflow_journal.go, from the Workflow tool_use/tool_result pair in
//     the main session transcript.
//   - internal C2: once a task ID is known, poll the corresponding tasks
//     output file (mtime-gated, size-capped) and decode only the
//     `workflowProgress` array — never `result`/`logs`.
//   - internal C3: config toggle, provider guard, and folding the decoded
//     detail into the broadcast WorkflowProgress without disturbing the
//     journal/VT-owned Done/Total/Running authority.
//
// It intentionally does not replace anything in workflow_journal.go; it only
// adds a second, independent one-shot-then-polling timer chain (same pattern
// as workflowJournalTimer/workflowScanTimer) triggered from the point where
// the journal watcher links a new wf_<runid> run.
const (
	// workflowTaskDetailMaxAttempts caps the one-shot taskId resolution retry
	// chain. Claude Code's tool_result write can lag slightly behind the
	// Workflow tool_use appearing in the transcript (C1 発見2), so a short
	// bounded retry absorbs that lag without polling forever.
	workflowTaskDetailMaxAttempts   = 5
	workflowTaskDetailRetryInterval = time.Second

	// workflowTaskDetailPollInterval governs the ongoing tasks-output poll
	// once a taskId has been resolved. Deliberately looser than the 1s
	// journal poll: the output file can approach 1 MB on a large audit run
	// and is rewritten wholesale on every update (C1 発見3), so re-parsing it
	// every second would be wasted work.
	workflowTaskDetailPollInterval = 4 * time.Second

	// workflowTaskDetailReadBytesMax/RecordsMax/ReadTimeBudget bound the tail
	// scan of the main session transcript used to resolve a taskId. This
	// mirrors agent_chat_parse.go's budgeted tail reader rather than scanning
	// the whole transcript, which can reach tens of thousands of lines.
	workflowTaskDetailReadBytesMax   = 2 * 1024 * 1024
	workflowTaskDetailReadRecordsMax = 500
	workflowTaskDetailReadTimeBudget = 100 * time.Millisecond

	// workflowTaskDetailOutputSizeMax bounds the tasks output file read. C1
	// measured 963 KB on a 14-agent audit run; 32 MB leaves ample headroom
	// for far larger runs while still refusing to buffer an unbounded file.
	workflowTaskDetailOutputSizeMax = 32 * 1024 * 1024

	// workflowTaskDetailTextMax re-truncates the already-short preview/
	// summary fields Claude Code writes, as a second independent guard
	// (belt-and-suspenders per the C2 plan).
	workflowTaskDetailTextMax = 500
)

var (
	workflowTaskIDRe        = regexp.MustCompile(`Task ID:\s*(\S+)`)
	workflowTranscriptDirRe = regexp.MustCompile(`Transcript dir:\s*(.+)`)
)

// workflowTaskDetailFileState is the mtime-gated cache for one tasks output
// file. Entries never contains a result/logs field: workflowTaskOutputFile
// below simply does not declare them, so encoding/json silently drops them.
type workflowTaskDetailFileState struct {
	Path       string
	ModTime    time.Time
	Loaded     bool
	Entries    []workflowTaskOutputEntry
	AgentCount int
}

// workflowTaskOutputFile is the intentionally narrow decode shape for a
// Claude Code tasks/<taskId>.output file. It must never gain a Result or Logs
// field: those bodies are opaque, potentially sensitive script output that
// the Hub does not read, log, or forward (see C1 発見1).
type workflowTaskOutputFile struct {
	AgentCount       int                       `json:"agentCount,omitempty"`
	WorkflowProgress []workflowTaskOutputEntry `json:"workflowProgress"`
}

// workflowTaskOutputEntry covers both element shapes ("workflow_phase" and
// "workflow_agent") that share the workflowProgress array; unused fields for
// a given Type are simply left zero. Field set is exactly the schema recorded
// in C1's 実測サンプル.
type workflowTaskOutputEntry struct {
	Type            string `json:"type"`
	Index           int    `json:"index"`
	Title           string `json:"title,omitempty"`
	Label           string `json:"label,omitempty"`
	PhaseIndex      int    `json:"phaseIndex,omitempty"`
	PhaseTitle      string `json:"phaseTitle,omitempty"`
	AgentID         string `json:"agentId,omitempty"`
	Model           string `json:"model,omitempty"`
	State           string `json:"state,omitempty"`
	StartedAt       int64  `json:"startedAt,omitempty"`
	QueuedAt        int64  `json:"queuedAt,omitempty"`
	Attempt         int    `json:"attempt,omitempty"`
	LastToolName    string `json:"lastToolName,omitempty"`
	LastToolSummary string `json:"lastToolSummary,omitempty"`
	PromptPreview   string `json:"promptPreview,omitempty"`
	LastProgressAt  int64  `json:"lastProgressAt,omitempty"`
	Tokens          int    `json:"tokens,omitempty"`
	ToolCalls       int    `json:"toolCalls,omitempty"`
	DurationMs      int64  `json:"durationMs,omitempty"`
	ResultPreview   string `json:"resultPreview,omitempty"`
}

// resolveWorkflowTaskID scans the tail of a Claude main session transcript,
// newest record first, for a completed Workflow tool_use/tool_result pair
// whose "Transcript dir:" line matches wfDir exactly (C1 発見2). It never
// reads the whole transcript: readAgentChatTailPageWithBudget bounds both the
// byte window and the wall-clock deadline, the same reader agent_chat_parse.go
// uses for its tail page. A budget that expires before any usable window is
// assembled simply yields no records, which this function reports as
// "unresolved" rather than an error — the caller's retry chain covers that.
func resolveWorkflowTaskID(transcriptPath, wfDir string, budget agentChatReadBudget) (string, bool) {
	wfDir = strings.TrimSpace(wfDir)
	if transcriptPath == "" || wfDir == "" {
		return "", false
	}
	records, _, err := readAgentChatTailPageWithBudget(transcriptPath, workflowTaskDetailReadRecordsMax, -1, budget)
	if err != nil || len(records) == 0 {
		return "", false
	}

	type toolResultMatch struct {
		taskID string
		dir    string
	}
	results := make(map[string]toolResultMatch)

	// records is newest-first. A tool_result always appears after its
	// tool_use in the transcript, so scanning newest-first guarantees any
	// tool_result for a given tool_use_id has already been indexed into
	// results by the time that tool_use line is reached.
	for _, rec := range records {
		var line claudeTranscriptLine
		if json.Unmarshal(rec.line, &line) != nil {
			continue
		}
		var blocks []claudeContentBlock
		if json.Unmarshal(line.Message.Content, &blocks) != nil {
			continue
		}
		switch line.Type {
		case "user":
			for _, block := range blocks {
				if block.Type != "tool_result" || block.ToolUseID == "" {
					continue
				}
				text := extractAgentChatText(block.Content)
				if text == "" {
					text = block.Text
				}
				if text == "" {
					continue
				}
				var match toolResultMatch
				if m := workflowTaskIDRe.FindStringSubmatch(text); m != nil {
					match.taskID = strings.TrimSpace(m[1])
				}
				if m := workflowTranscriptDirRe.FindStringSubmatch(text); m != nil {
					match.dir = strings.TrimSpace(m[1])
				}
				if match.taskID != "" && match.dir != "" {
					results[block.ToolUseID] = match
				}
			}
		case "assistant":
			for _, block := range blocks {
				if block.Type != "tool_use" || block.Name != "Workflow" || block.ID == "" {
					continue
				}
				if match, ok := results[block.ID]; ok && match.dir == wfDir {
					return match.taskID, true
				}
			}
		}
	}
	return "", false
}

// pollWorkflowTaskDetailFile re-reads and re-decodes path only when its mtime
// differs from prior (or it was never loaded). Claude Code rewrites the whole
// tasks output file on every update rather than appending (C1 発見3), so
// unlike journal.jsonl this needs no byte offset — a changed mtime means "read
// it all again", an unchanged mtime means "nothing to do". The returned bool
// is true exactly when a re-parse happened, so callers/tests can verify the
// mtime gate without a full timer harness.
func pollWorkflowTaskDetailFile(path string, prior workflowTaskDetailFileState) (workflowTaskDetailFileState, bool, error) {
	info, err := os.Stat(path)
	if err != nil {
		return prior, false, err
	}
	if info.IsDir() {
		return prior, false, fmt.Errorf("workflow task output path is a directory")
	}
	if prior.Loaded && prior.Path == path && prior.ModTime.Equal(info.ModTime()) {
		return prior, false, nil
	}
	if info.Size() > workflowTaskDetailOutputSizeMax {
		// Oversized: skip reading and keep whatever was previously loaded (if
		// any). Do not mark this mtime as consumed, so a later poll retries
		// once the file shrinks or budget assumptions are revisited.
		stale := prior
		stale.Path = path
		return stale, false, nil
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path is derived from the local Claude tasks-output root, not user input.
	if err != nil {
		return prior, false, err
	}
	var file workflowTaskOutputFile
	if err := json.Unmarshal(data, &file); err != nil {
		return prior, false, err
	}
	return workflowTaskDetailFileState{
		Path:       path,
		ModTime:    info.ModTime(),
		Loaded:     true,
		Entries:    file.WorkflowProgress,
		AgentCount: file.AgentCount,
	}, true, nil
}

// buildWorkflowTaskDetailProgress turns the flat workflowProgress array into
// the same Phases/WfAgent shape the VT/journal path produces, so the overlay
// in workflowProgressForBroadcastLocked can swap Phases wholesale. Phase
// order follows phase Index; an agent whose PhaseIndex has no declared
// workflow_phase entry gets a synthetic phase from its own PhaseTitle rather
// than being dropped.
func buildWorkflowTaskDetailProgress(entries []workflowTaskOutputEntry) *proto.WorkflowProgress {
	if len(entries) == 0 {
		return nil
	}
	type phaseBuild struct {
		title  string
		agents []proto.WfAgent
	}
	phases := make(map[int]*phaseBuild)
	var order []int

	for _, e := range entries {
		if e.Type != "workflow_phase" {
			continue
		}
		if _, exists := phases[e.Index]; !exists {
			order = append(order, e.Index)
		}
		phases[e.Index] = &phaseBuild{title: e.Title}
	}
	for _, e := range entries {
		if e.Type != "workflow_agent" {
			continue
		}
		pb, exists := phases[e.PhaseIndex]
		if !exists {
			pb = &phaseBuild{title: e.PhaseTitle}
			phases[e.PhaseIndex] = pb
			order = append(order, e.PhaseIndex)
		}
		pb.agents = append(pb.agents, proto.WfAgent{
			Label: e.Label,
			State: e.State,
			Detail: &proto.WfAgentDetail{
				Model:           e.Model,
				StartedAt:       e.StartedAt,
				LastProgressAt:  e.LastProgressAt,
				DurationMs:      e.DurationMs,
				Tokens:          e.Tokens,
				ToolCalls:       e.ToolCalls,
				LastToolName:    truncateWorkflowText(e.LastToolName, workflowTaskDetailTextMax),
				LastToolSummary: truncateWorkflowText(e.LastToolSummary, workflowTaskDetailTextMax),
				PromptPreview:   truncateWorkflowText(e.PromptPreview, workflowTaskDetailTextMax),
				ResultPreview:   truncateWorkflowText(e.ResultPreview, workflowTaskDetailTextMax),
			},
		})
	}
	if len(order) == 0 {
		return nil
	}
	sort.Ints(order)
	p := &proto.WorkflowProgress{Source: "task-output", TaskDetailSource: "task-output"}
	for _, idx := range order {
		pb := phases[idx]
		p.Phases = append(p.Phases, proto.WfPhase{Title: pb.title, Agents: pb.agents})
	}
	return p
}

// applyWorkflowTaskDetailOverlay replaces out.Phases with the richer
// tasks-output-derived tree and stamps TaskDetailSource. It deliberately does
// not touch Done/Total/Running/Percent/Settled/SettledBy: those stay owned by
// composeWorkflowProgress's existing VT/journal authority (親 plan D1).
func applyWorkflowTaskDetailOverlay(out, detail *proto.WorkflowProgress) {
	if out == nil || detail == nil {
		return
	}
	out.Phases = detail.Phases
	out.TaskDetailSource = detail.TaskDetailSource
}

// newWorkflowJournalRunDir returns the wf_<runid> directory (the parent of a
// journal.jsonl path) for the most recently modified file key present in
// current but absent from previous. It is used only to trigger a one-shot
// task-detail resolution attempt when the journal watcher observes a run it
// was not previously tracking; it has no effect on journal counting.
func newWorkflowJournalRunDir(current, previous map[string]workflowJournalFileState) (string, bool) {
	var bestPath string
	var bestState workflowJournalFileState
	found := false
	for path, state := range current {
		if _, existed := previous[path]; existed {
			continue
		}
		if !found || state.ModTime.After(bestState.ModTime) || (state.ModTime.Equal(bestState.ModTime) && path > bestPath) {
			bestPath, bestState, found = path, state, true
		}
	}
	if !found {
		return "", false
	}
	return filepath.Dir(bestPath), true
}

func (s *Server) workflowTaskDetailEnabled() bool {
	if s == nil || s.cfg == nil {
		return false
	}
	return s.snapshotCfg().Workflow.TaskDetailEnabled
}

// startWorkflowTaskDetailLocked begins the one-shot taskId resolution chain
// for a newly linked wf_<runid> run. Called with sessionsMu held, from the
// same place workflow_journal.go links ses.workflowJournalSessionDir/Files.
// It is a no-op once this session has been marked unavailable: 5 failed
// resolution attempts means the transcript did not match the expected
// Workflow tool_use/tool_result shape at all (a version/format mismatch, not
// a timing fluke), and retrying on every subsequent Workflow call in the same
// session would just fail the same way (plan 内部C1 "このセッションでは提供しない").
func (s *Server) startWorkflowTaskDetailLocked(id int, ses *session, now time.Time, sessionDir, wfDir string) {
	if ses == nil || ses.Provider != "claude" || ses.taskDetailUnavailable {
		return
	}
	if ses.taskDetailTimer != nil {
		ses.taskDetailTimer.Stop()
		ses.taskDetailTimer = nil
	}
	ses.taskDetailGeneration++
	ses.taskDetailWfDir = wfDir
	ses.taskDetailSessionUUID = filepath.Base(sessionDir)
	ses.taskDetailTaskID = ""
	ses.taskDetailResolveAttempts = 0
	ses.taskDetailFileState = workflowTaskDetailFileState{}
	ses.taskDetailProgress = nil
	generation := ses.taskDetailGeneration
	expected := ses
	ses.taskDetailTimer = time.AfterFunc(0, func() { s.runWorkflowTaskDetailResolve(id, expected, generation) })
}

// stopWorkflowTaskDetailLocked cancels any in-flight resolve/poll timer and
// clears per-run state. It does not clear taskDetailUnavailable: that flag is
// sticky for the lifetime of the hub session by design (see
// startWorkflowTaskDetailLocked).
func (s *Server) stopWorkflowTaskDetailLocked(ses *session) {
	if ses == nil {
		return
	}
	if ses.taskDetailTimer != nil {
		ses.taskDetailTimer.Stop()
		ses.taskDetailTimer = nil
	}
	ses.taskDetailGeneration++
	ses.taskDetailWfDir = ""
	ses.taskDetailSessionUUID = ""
	ses.taskDetailTaskID = ""
	ses.taskDetailResolveAttempts = 0
	ses.taskDetailFileState = workflowTaskDetailFileState{}
	ses.taskDetailProgress = nil
}

func (s *Server) runWorkflowTaskDetailResolve(id int, expected *session, generation uint64) {
	if !s.workflowTaskDetailEnabled() {
		s.sessionsMu.Lock()
		if ses := s.sessions[id]; ses != nil && ses == expected {
			s.stopWorkflowTaskDetailLocked(ses)
		}
		s.sessionsMu.Unlock()
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.taskDetailGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.taskDetailTimer = nil
	ses.taskDetailResolveAttempts++
	attempt := ses.taskDetailResolveAttempts
	sessionDir := ses.workflowJournalSessionDir
	wfDir := ses.taskDetailWfDir
	s.sessionsMu.Unlock()

	var taskID string
	var ok bool
	if sessionDir != "" && wfDir != "" {
		transcriptPath := sessionDir + ".jsonl"
		budget := agentChatReadBudget{
			MaxBytes:   workflowTaskDetailReadBytesMax,
			MaxRecords: workflowTaskDetailReadRecordsMax,
			Deadline:   time.Now().Add(workflowTaskDetailReadTimeBudget),
		}
		taskID, ok = resolveWorkflowTaskID(transcriptPath, wfDir, budget)
	}

	s.sessionsMu.Lock()
	ses = s.sessions[id]
	if ses == nil || ses != expected || ses.taskDetailGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	switch {
	case ok:
		ses.taskDetailTaskID = taskID
		ses.taskDetailTimer = time.AfterFunc(0, func() { s.runWorkflowTaskDetailPoll(id, expected, generation) })
	case attempt >= workflowTaskDetailMaxAttempts:
		ses.taskDetailUnavailable = true
	default:
		ses.taskDetailTimer = time.AfterFunc(workflowTaskDetailRetryInterval, func() { s.runWorkflowTaskDetailResolve(id, expected, generation) })
	}
	s.sessionsMu.Unlock()
}

func (s *Server) runWorkflowTaskDetailPoll(id int, expected *session, generation uint64) {
	if !s.workflowTaskDetailEnabled() {
		s.sessionsMu.Lock()
		if ses := s.sessions[id]; ses != nil && ses == expected {
			s.stopWorkflowTaskDetailLocked(ses)
		}
		s.sessionsMu.Unlock()
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.Provider != "claude" || ses.taskDetailGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.taskDetailTimer = nil
	cwd := ses.CWD
	taskID := ses.taskDetailTaskID
	sessionUUID := ses.taskDetailSessionUUID
	prior := ses.taskDetailFileState
	s.sessionsMu.Unlock()

	outputPath := filepath.Join(os.TempDir(), "claude", claudeProjectDirName(cwd), sessionUUID, "tasks", taskID+".output")
	newState, _, _ := pollWorkflowTaskDetailFile(outputPath, prior)
	var progress *proto.WorkflowProgress
	if newState.Loaded {
		progress = buildWorkflowTaskDetailProgress(newState.Entries)
	}

	now := time.Now()
	s.sessionsMu.Lock()
	ses = s.sessions[id]
	if ses == nil || ses != expected || ses.taskDetailGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.taskDetailFileState = newState
	if progress != nil {
		ses.taskDetailProgress = progress
	}
	// journalCounts.Settled is the same authoritative settle signal
	// workflowProgressForBroadcastLocked already relies on (D1). Once the
	// workflow has settled, this poll's read above already captured the final
	// tasks-output snapshot (Claude Code stops rewriting the file once every
	// agent finishes, well before the journal's 10s settle-quiet window
	// elapses), so there is nothing left to gain by rescheduling.
	if !ses.journalCounts.Settled {
		ses.taskDetailTimer = time.AfterFunc(workflowTaskDetailPollInterval, func() { s.runWorkflowTaskDetailPoll(id, expected, generation) })
	}
	out, shouldBroadcast, shouldNotify := s.workflowProgressForBroadcastLocked(ses, now)
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
	if shouldNotify {
		s.notifyWorkflowCompletionPush(id, out)
	}
}
